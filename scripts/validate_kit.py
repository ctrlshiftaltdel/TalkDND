#!/usr/bin/env python3
"""Static validation for the Agent-Orchestration kit (core + local/github editions)."""
from pathlib import Path
import argparse, re, sys

ROOT = Path(__file__).resolve().parents[1]
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--project-ready', action='store_true', help='fail while ORCHESTRATION-PROJECT.md still has FILL_IN placeholders')
args = parser.parse_args()
# Single source of truth for the shippable kit. This list has two jobs at once:
#   1. the structurally required files of this repository, and
#   2. the file manifest of the release package built by scripts/build_release.py.
# scripts/build_release.py parses THIS list to decide what to package, so there is
# no second manifest to maintain. Keep it in sync whenever a versioned artifact is
# added or removed. Files that must not ship (docs/, release/, the release builder
# itself, .orchestration/, .worktrees/, .workbuddy/) are simply not listed here.
REQUIRED = [
    'AGENT-BOOTSTRAP.md', 'AGENT-ORCHESTRATION-ACCEPTANCE.md', 'AGENT-ORCHESTRATION.md',
    'AGENTS.md', 'LICENSE.txt', 'ORCHESTRATION-PROJECT.md', 'README.md', 'VERSION.txt',
    'editions/github/ACTIVE-TASK-REGISTRY.md', 'editions/github/ISSUE-TEMPLATE.md',
    'editions/github/MASTER-MULTI-SESSION.md', 'editions/github/MASTER-SINGLE-SESSION.md',
    'editions/github/MASTER.md', 'editions/github/PR-TEMPLATE.md', 'editions/github/README.md',
    'editions/github/START-EXECUTION.md', 'editions/github/START-MASTER-EXISTING-PROJECT.md',
    'editions/github/START-MASTER-NEW-PROJECT.md',
    'editions/local/MASTER-MULTI-SESSION.md', 'editions/local/MASTER-SINGLE-SESSION.md',
    'editions/local/MASTER.md', 'editions/local/README.md', 'editions/local/START-EXECUTION.md',
    'editions/local/START-MASTER-EXISTING-PROJECT.md', 'editions/local/START-MASTER-NEW-PROJECT.md',
    'prompts/START-CHARTER.md', 'prompts/START-REVIEWER.md',
    'scripts/orchestrator.py', 'scripts/validate_kit.py',
    'templates/DESIGN.md', 'templates/ORCHESTRATION-PROJECT.template.md', 'templates/PLAN.md',
    'templates/PROJECT-CHARTER.md', 'templates/REVIEW-CHECKLIST.md', 'templates/TASK-PACKET.md',
    'templates/gitignore.snippet',
    'tests/test_orchestrator.py',
]
errors=[]
for rel in REQUIRED:
    if not (ROOT/rel).is_file(): errors.append(f'missing required file: {rel}')

# The two editions must expose the same public file set; github additionally ships
# three templates. Comparing the *intersection* catches a file that exists on only
# one side, while comparing each side against its expected set catches strays.
COMMON_EDITION_FILES = (
    'MASTER-MULTI-SESSION.md', 'MASTER-SINGLE-SESSION.md', 'MASTER.md', 'README.md',
    'START-EXECUTION.md', 'START-MASTER-EXISTING-PROJECT.md', 'START-MASTER-NEW-PROJECT.md',
)
GITHUB_ONLY_FILES = ('ACTIVE-TASK-REGISTRY.md', 'ISSUE-TEMPLATE.md', 'PR-TEMPLATE.md')
edition_names={}
for edition in ('local', 'github'):
    directory = ROOT/'editions'/edition
    if not directory.is_dir():
        errors.append(f'missing edition directory: editions/{edition}')
        edition_names[edition] = set()
        continue
    actual = {p.name for p in directory.iterdir() if p.is_file()}
    edition_names[edition] = actual
    expected = set(COMMON_EDITION_FILES) | (set(GITHUB_ONLY_FILES) if edition == 'github' else set())
    for name in sorted(expected - actual):
        errors.append(f'editions/{edition} is missing file: {name}')
    for name in sorted(actual - expected):
        errors.append(f'editions/{edition} contains unexpected file: {name}')
if edition_names.get('local') is not None and edition_names.get('github') is not None:
    shared = edition_names['local'] & edition_names['github']
    if shared != set(COMMON_EDITION_FILES):
        errors.append('editions/local and editions/github disagree on the shared file set: '
                      f'intersection={sorted(shared)}')
for name in GITHUB_ONLY_FILES:
    if not (ROOT/'editions'/'github'/name).is_file():
        errors.append(f'missing github-only template: editions/github/{name}')

# The core orchestration document must really define the governance concepts it is
# supposed to own: PR_OPEN (github-only state), merge gates G1-G6 and runtime
# capability contracts C1-C4. This is a content check, not a self-certification.
core_path = ROOT/'AGENT-ORCHESTRATION.md'
if core_path.is_file():
    core_text = core_path.read_text(encoding='utf-8')
    core_needles = ['PR_OPEN'] + [f'G{i}' for i in range(1, 7)] + [f'C{i}' for i in range(1, 5)]
    for needle in core_needles:
        if needle not in core_text:
            errors.append(f'AGENT-ORCHESTRATION.md is missing required concept: {needle}')

# The edition selector is the routing key for the whole kit. It must be present and
# explicit: silently defaulting would run a project on the wrong task carrier.
config_path = ROOT/'ORCHESTRATION-PROJECT.md'
if config_path.is_file():
    config_text = config_path.read_text(encoding='utf-8')
    match = re.search(r'^\s*[-*]\s*`?edition`?\s*[:：]\s*`?([A-Za-z0-9_-]+)`?', config_text, re.M)
    if not match:
        errors.append('ORCHESTRATION-PROJECT.md is missing the `edition` key '
                      "(expected `edition`: `local` or `github`)")
    else:
        edition_value = match.group(1)
        if edition_value not in ('local', 'github'):
            errors.append("ORCHESTRATION-PROJECT.md `edition` must be 'local' or 'github', "
                          f"got {edition_value!r}")

# Governance concepts are only meaningful in documentation. Scan document-class
# files (*.md / *.txt) and skip code dirs and binaries so the check cannot be
# satisfied by the validator's own source or by non-document assets.
DOC_SUFFIXES = ('.md', '.txt')
EXCLUDED_DIRS = {'scripts', '.orchestration', '.git', '__pycache__'}
doc_texts=[]
for p in ROOT.rglob('*'):
    if not p.is_file(): continue
    rel=p.relative_to(ROOT)
    if rel.suffix.lower() not in DOC_SUFFIXES: continue
    if EXCLUDED_DIRS & set(rel.parts): continue
    raw=p.read_text(encoding='utf-8', errors='replace')
    if '\x00' in raw: continue  # skip binary payloads
    doc_texts.append(raw)
text_all='\n'.join(doc_texts)
for needle in ['PLANNED', 'DISPATCHED', 'MASTER_REVIEW', 'APPROVED', 'VERIFIED', 'NEEDS_HUMAN_DECISION']:
    if needle not in text_all: errors.append(f'missing expected governance concept: {needle}')
# Relative Markdown links must resolve. Ignore anchors and external URLs.
for md in ROOT.rglob('*.md'):
    content=md.read_text(encoding='utf-8')
    for target in re.findall(r'\[[^\]]*\]\(([^)]+)\)', content):
        if target.startswith(('http://','https://','mailto:','#')): continue
        target=target.split('#',1)[0]
        if target and not (md.parent/target).exists(): errors.append(f'{md.relative_to(ROOT)}: broken relative link: {target}')
if args.project_ready:
    cfg=(ROOT/'ORCHESTRATION-PROJECT.md').read_text(encoding='utf-8')
    missing=cfg.count('FILL_IN')
    if missing: errors.append(f'ORCHESTRATION-PROJECT.md has {missing} FILL_IN placeholders')
if errors:
    print('VALIDATION FAILED')
    print('\n'.join(f'- {e}' for e in errors))
    sys.exit(1)
print(f'Kit validation passed ({len(REQUIRED)} required files checked; '
      f'editions local/github share {len(COMMON_EDITION_FILES)} files, '
      f'github adds {len(GITHUB_ONLY_FILES)} templates).')
