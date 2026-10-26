"""Python dependency adapter: static `ast` scanning, standard library only.

What it can prove: a module-level or function-level `import` / `from ... import`
statement is a static dependency on the resolved project file. Those edges feed
NO_CYCLE / FORBIDDEN_EDGE / LAYER_DIRECTION / PRIVATE_ACCESS.

What it refuses to claim:

- dynamic imports (`importlib.import_module(...)`, `__import__`, `exec`-built
  module names) are reported as `UNKNOWN` even when the argument is a constant
  string, because whether the call executes is a runtime property;
- an import whose target cannot be resolved inside the declared module paths is
  reported as `UNKNOWN`, never silently dropped;
- a file that does not parse is reported as `UNKNOWN`, never treated as
  dependency-free.

So `UNKNOWN` is a first-class outcome and is never folded into a pass.
"""
from __future__ import annotations

import ast
import re
from pathlib import Path

SKIP_DIRS = frozenset({
    '.git', '.hg', '.svn', '.worktrees', '.orchestration', '.venv', 'venv',
    'node_modules', '__pycache__', 'build', 'dist', 'target', '.mypy_cache',
    '.pytest_cache', '.ruff_cache', '.local',
})

DOTTED_RE = re.compile(r'^[A-Za-z_][A-Za-z0-9_]*(\.[A-Za-z_][A-Za-z0-9_]*)*$')


def iter_files(project_root, suffix):
    """(relative posix path, absolute path) for every *suffix* file, skipping noise."""
    root = Path(project_root)
    for path in sorted(root.rglob('*' + suffix)):
        if not path.is_file():
            continue
        relative = path.relative_to(root)
        if SKIP_DIRS & set(relative.parts):
            continue
        yield relative.as_posix(), path


def _dotted(relative_path):
    """`src/app/main.py` -> `src.app.main`; `pkg/__init__.py` -> `pkg`."""
    parts = relative_path[:-len('.py')].split('/')
    if parts and parts[-1] == '__init__':
        parts = parts[:-1]
    return '.'.join(part for part in parts if part)


class ImportIndex:
    """Resolve dotted module names to project files.

    A repository usually has a source root (`src/`, `lib/`, the repo root), so a
    contract module at `src/app/` is imported as `app.main` or `src.app.main`
    depending on how the project is packaged. Exact root-relative names win;
    otherwise a unique path suffix is accepted. An ambiguous suffix (two
    `utils.py` in different packages) resolves to nothing and the caller reports
    `UNKNOWN` instead of guessing.
    """

    def __init__(self, files):
        self.exact = {}
        self.suffixes = {}
        ambiguous = set()
        for relative in files:
            dotted = _dotted(relative)
            if not dotted:
                continue
            self.exact.setdefault(dotted, relative)
            parts = dotted.split('.')
            for start in range(1, len(parts)):
                tail = '.'.join(parts[start:])
                if tail in self.suffixes and self.suffixes[tail] != relative:
                    ambiguous.add(tail)
                else:
                    self.suffixes.setdefault(tail, relative)
        for name in ambiguous:
            self.suffixes.pop(name, None)
        self.ambiguous = ambiguous

    def resolve(self, dotted):
        if not dotted:
            return None
        return self.exact.get(dotted) or self.suffixes.get(dotted)


class _Collector(ast.NodeVisitor):
    def __init__(self, relative, package):
        self.relative = relative
        self.package = package
        self.imports = []   # (dotted base, [alias names] | None, lineno, conditional, text)
        self.dynamic = []   # (lineno, message)
        self.conditional = 0

    # -- conditional contexts ------------------------------------------------
    def _visit_conditional(self, node):
        self.conditional += 1
        self.generic_visit(node)
        self.conditional -= 1

    visit_If = _visit_conditional
    visit_Try = _visit_conditional
    visit_TryStar = _visit_conditional
    visit_Match = _visit_conditional

    # -- imports -------------------------------------------------------------
    def visit_Import(self, node):
        for alias in node.names:
            self.imports.append((alias.name, None, node.lineno,
                                 self.conditional > 0, f'import {alias.name}'))

    def visit_ImportFrom(self, node):
        base = self._relative_base(node)
        names = [alias.name for alias in node.names]
        text = 'from ' + ('.' * node.level) + (node.module or '') \
            + ' import ' + ', '.join(names)
        self.imports.append((base, names, node.lineno, self.conditional > 0, text))

    def _relative_base(self, node):
        if not node.level:
            return node.module or ''
        package = self.package[:]
        drop = node.level - 1
        if drop > len(package):
            return node.module or ''
        if drop:
            package = package[:len(package) - drop]
        parts = package + ([node.module] if node.module else [])
        return '.'.join(parts)

    # -- dynamic imports -----------------------------------------------------
    def visit_Call(self, node):
        message = self._dynamic_call(node)
        if message:
            self.dynamic.append((node.lineno, message))
        self.generic_visit(node)

    def _dynamic_call(self, node):
        func = node.func
        name = None
        if isinstance(func, ast.Name):
            name = func.id
        elif isinstance(func, ast.Attribute):
            chain = []
            current = func
            while isinstance(current, ast.Attribute):
                chain.append(current.attr)
                current = current.value
            if isinstance(current, ast.Name):
                chain.append(current.id)
            name = '.'.join(reversed(chain))
        if name in ('__import__', 'importlib.import_module',
                    'importlib.util.spec_from_file_location',
                    'importlib.__import__'):
            literal = ''
            if node.args and isinstance(node.args[0], ast.Constant) \
                    and isinstance(node.args[0].value, str):
                literal = f' with constant argument {node.args[0].value!r}'
            return (f'dynamic import via {name}{literal}: the runtime target cannot be '
                    'proven statically, review this edge manually')
        return None


def package_parts(relative):
    parts = relative[:-len('.py')].split('/')
    if parts and parts[-1] == '__init__':
        parts = parts[:-1]
    else:
        parts = parts[:-1]
    return [part for part in parts if part]


def scan(project_root, internal_roots=()):
    """Scan every Python file; return (records, unknowns, info).

    records: {'source_file', 'line', 'target_file'|None, 'dotted', 'text',
              'conditional', 'dynamic'}
    unknowns: {'file', 'line', 'message'}
    info: {'files': int, 'unparsable': [rel, ...]}
    """
    files = list(iter_files(project_root, '.py'))
    relative_files = [relative for relative, _ in files]
    index = ImportIndex(relative_files)
    roots = {str(root).strip('/') for root in internal_roots if root}
    records, unknowns, unparsable = [], [], []

    for relative, path in files:
        try:
            source = path.read_text(encoding='utf-8')
            tree = ast.parse(source, filename=relative)
        except (OSError, UnicodeDecodeError, SyntaxError) as exc:
            unparsable.append(relative)
            unknowns.append({'file': relative, 'line': None,
                             'message': f'file could not be parsed, dependencies are UNKNOWN: {exc}'})
            continue
        collector = _Collector(relative, package_parts(relative))
        collector.visit(tree)
        for line, message in collector.dynamic:
            unknowns.append({'file': relative, 'line': line, 'message': message})
        for base, names, line, conditional, text in collector.imports:
            resolved = _resolve_import(index, base, names)
            if resolved:
                for target in resolved:
                    records.append({'source_file': relative, 'line': line,
                                    'target_file': target, 'dotted': base, 'text': text,
                                    'conditional': conditional, 'dynamic': False})
                continue
            # A dynamic call with a constant argument is already reported above;
            # here only static statements that look project-internal are flagged.
            first = (base or '').split('.')[0]
            if first and (first in roots or (names and not base)):
                unknowns.append({
                    'file': relative, 'line': line,
                    'message': f'unresolved import {text!r}: no project file matches '
                               f'{base!r}; the dependency is UNKNOWN, not absent',
                })
    return records, unknowns, {'files': len(files), 'unparsable': unparsable}


def _resolve_import(index, base, names):
    """Resolve one import statement to zero or more project files."""
    if names is None:  # plain `import a.b.c`
        target = index.resolve(base)
        return [target] if target else []
    targets = []
    for name in names:
        if name == '*':
            target = index.resolve(base)
            if target:
                targets.append(target)
            continue
        candidate = f'{base}.{name}' if base else name
        target = index.resolve(candidate)
        if target is None:
            target = index.resolve(base)
        if target and target not in targets:
            targets.append(target)
    return targets
