#!/usr/bin/env python3
"""Command line for the experience pipeline (#49).

Usage::

    python3 scripts/self_improvement/cli.py registry show
    python3 scripts/self_improvement/cli.py registry validate
    python3 scripts/self_improvement/cli.py registry add --source-id example \
        --repository owner/example --evidence-scope pull_request,issue,commit
    python3 scripts/self_improvement/cli.py registry disable --source-id example
    python3 scripts/self_improvement/cli.py registry remove --source-id example
    python3 scripts/self_improvement/cli.py scan --json
    python3 scripts/self_improvement/cli.py scan --offline
    python3 scripts/self_improvement/cli.py lesson list
    python3 scripts/self_improvement/cli.py lesson validate --file lesson.json

Registry mutations are the *only* write path for sources, and they are plain
configuration edits: no code path changes when a source is added, disabled or
removed. Scan output goes to ``.local/self-improvement/`` (git-ignored) by
default; ``--dry-run`` keeps the registry untouched.

Exit codes: ``0`` report produced / command succeeded, ``1`` configuration or
validation error, ``2`` usage error, ``3`` report produced but every enabled
source was ``BLOCKED`` / ``UNKNOWN`` / ``NOT_RUN`` (never treated as success).
"""
from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

if __package__ in (None, ''):  # executed as a script
    sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
    from self_improvement import collect, lessons, pipeline, sources  # noqa: E402
else:  # imported as a package
    from . import collect, lessons, pipeline, sources  # noqa: E402

ROOT = Path(__file__).resolve().parents[2]
DEFAULT_REGISTRY = ROOT / 'docs' / 'self-improvement' / 'sources.json'
DEFAULT_STATE_DIR = ROOT / '.local' / 'self-improvement'
EXIT_OK = 0
EXIT_CONFIG = 1
EXIT_USAGE = 2
EXIT_NO_SCAN = 3


def _registry_path(args):
    return Path(args.registry)


def _load(args):
    return sources.Registry.load(_registry_path(args))


def cmd_registry_show(args):
    registry = _load(args)
    payload = sources.summary(registry)
    if args.json:
        print(json.dumps(payload, ensure_ascii=False, indent=2))
        return EXIT_OK
    print(f"registry: {_registry_path(args)}")
    print(f"schema_version: {payload['schema_version']}  "
          f"sources: {payload['source_count']} (enabled {payload['enabled_count']})")
    for source in registry.sources:
        cursor = source.get('scan_cursor')
        state = 'never-scanned' if not cursor else (
            f"{cursor.get('last_scan_status')} @ {cursor.get('last_successful_scan_at')}")
        print(f"- {source['source_id']}: {source['repository']} "
              f"enabled={source['enabled']} scope={','.join(source['evidence_scope'])} "
              f"lookback={source['lookback_days']}d cursor={state}")
    return EXIT_OK


def cmd_registry_validate(args):
    try:
        registry = _load(args)
    except sources.RegistryError as exc:
        print(f'INVALID: {exc}', file=sys.stderr)
        return EXIT_CONFIG
    print(f'valid registry: {len(registry.sources)} source(s), '
          f'{len(registry.enabled_sources())} enabled')
    return EXIT_OK


def cmd_registry_add(args):
    scope = [item.strip() for item in args.evidence_scope.split(',') if item.strip()]
    try:
        source = sources.make_source(
            args.source_id, args.repository, evidence_scope=scope,
            lookback_days=args.lookback_days, source_type=args.source_type,
            access_mode=args.access_mode, enabled=not args.disabled,
            inclusion_filters=args.include or [], exclusion_filters=args.exclude or [],
            owner_approved_notes=args.notes)
        registry = _load(args)
        registry = sources.add_source(registry, source, updated_at=_now())
        registry.save()
    except sources.RegistryError as exc:
        print(f'INVALID: {exc}', file=sys.stderr)
        return EXIT_CONFIG
    print(f'added {args.source_id} ({args.repository}); registry saved to {_registry_path(args)}')
    return EXIT_OK


def cmd_registry_toggle(args, enable):
    try:
        registry = _load(args)
        registry = (sources.enable_source(registry, args.source_id) if enable
                    else sources.disable_source(registry, args.source_id))
        registry.save(updated_at=_now())
    except sources.RegistryError as exc:
        print(f'INVALID: {exc}', file=sys.stderr)
        return EXIT_CONFIG
    print(f'{"enabled" if enable else "disabled"} {args.source_id}')
    return EXIT_OK


def cmd_registry_remove(args):
    try:
        registry = _load(args)
        registry = sources.remove_source(registry, args.source_id)
        registry.save(updated_at=_now())
    except sources.RegistryError as exc:
        print(f'INVALID: {exc}', file=sys.stderr)
        return EXIT_CONFIG
    print(f'removed {args.source_id}; {len(registry.sources)} source(s) remain. '
          'Existing lessons and decisions are kept.')
    return EXIT_OK


def cmd_scan(args):
    try:
        registry = _load(args)
    except sources.RegistryError as exc:
        print(f'CONFIG ERROR: {exc}', file=sys.stderr)
        return EXIT_CONFIG
    full_registry = registry
    if args.source:
        missing = [source_id for source_id in args.source if registry.get(source_id) is None]
        if missing:
            print(f'CONFIG ERROR: unknown source_id(s): {", ".join(missing)}', file=sys.stderr)
            return EXIT_CONFIG
        selected = [source for source in registry.sources
                    if source['source_id'] in set(args.source)]
        registry = registry.with_sources(selected)
    analyses = {}
    if args.analysis:
        try:
            analyses = _load_analyses(Path(args.analysis))
        except (OSError, ValueError, KeyError) as exc:
            print(f'CONFIG ERROR: analysis file: {exc}', file=sys.stderr)
            return EXIT_CONFIG
    store_path = Path(args.store)
    store = lessons.LessonStore.load(store_path)
    runner = collect.SubprocessRunner()
    try:
        outcome = pipeline.run_scan(registry, store, runner, now=args.now,
                                    offline=args.offline, max_pages=args.max_pages,
                                    analyses=analyses)
    except (collect.CollectionError, sources.RegistryError, lessons.LessonError) as exc:
        print(f'SCAN FAILED: {exc}', file=sys.stderr)
        return EXIT_CONFIG
    report = outcome.report
    report_dir = Path(args.report_dir)
    stamp = report['generated_at'].replace(':', '').replace('-', '')
    json_path = pipeline.write_report(report, report_dir / f'scan-{stamp}.json')
    md_path = pipeline.write_markdown(report, report_dir / f'scan-{stamp}.md')
    if not args.dry_run:
        store.save(store_path)
        if not args.offline:
            persisted = full_registry
            for scanned in outcome.registry.sources:
                cursor = scanned.get('scan_cursor')
                if cursor is not None:
                    persisted = sources.update_cursor(persisted, scanned['source_id'], cursor)
            persisted.save(updated_at=_now())
    if args.json:
        print(json.dumps(outcome.as_dict(), ensure_ascii=False, indent=2))
    else:
        print(lessons.render_markdown(report))
        print(f'report: {json_path}')
        print(f'report: {md_path}')
    counts = report['counts']
    if args.offline:
        return EXIT_OK
    if counts['sources_enabled'] and not (counts['sources_ok'] or counts['sources_partial']):
        return EXIT_NO_SCAN
    return EXIT_OK


def cmd_lesson_list(args):
    store = lessons.LessonStore.load(Path(args.store))
    records = store.all()
    if args.json:
        print(json.dumps(records, ensure_ascii=False, indent=2))
        return EXIT_OK
    if not records:
        print('no lessons recorded')
        return EXIT_OK
    for lesson in records:
        can_task, reason = lessons.can_generate_task(lesson)
        print(f"{lesson['lesson_id']} [{lesson['content_class']}/{lesson['status']}/"
              f"{lesson['evidence_level']}] {lesson['title']}")
        print(f"  evidence: {', '.join(lesson['source']['evidence_urls'])}")
        print(f"  task-eligible: {'yes' if can_task else 'no — ' + reason}")
    return EXIT_OK


def cmd_lesson_validate(args):
    try:
        data = json.loads(Path(args.file).read_text(encoding='utf-8'))
    except (OSError, json.JSONDecodeError) as exc:
        print(f'CONFIG ERROR: {exc}', file=sys.stderr)
        return EXIT_CONFIG
    records = data.get('lessons', data) if isinstance(data, dict) else data
    if isinstance(records, dict):
        records = [records]
    errors = []
    for index, record in enumerate(records):
        for error in lessons.validate_lesson(record):
            errors.append({'lesson_index': index, **error})
    if errors:
        print(json.dumps({'valid': False, 'errors': errors}, ensure_ascii=False, indent=2),
              file=sys.stderr)
        return EXIT_CONFIG
    print(f'valid: {len(records)} lesson(s)')
    return EXIT_OK


def _load_analyses(path):
    data = json.loads(path.read_text(encoding='utf-8'))
    if not isinstance(data, dict):
        raise ValueError('expected an object mapping evidence_url -> analysis')
    return data


def _now():
    return lessons.now_utc()


def build_parser():
    parser = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--registry', default=str(DEFAULT_REGISTRY),
                        help='source registry path (default: docs/self-improvement/sources.json)')
    parser.add_argument('--store', default=str(DEFAULT_STATE_DIR / 'lessons.json'),
                        help='lesson store path (default: .local/self-improvement/lessons.json)')
    subparsers = parser.add_subparsers(dest='command', required=True)

    registry = subparsers.add_parser('registry', help='inspect or edit the source registry')
    registry_sub = registry.add_subparsers(dest='registry_command', required=True)

    show = registry_sub.add_parser('show', help='print the registry summary')
    show.add_argument('--json', action='store_true')
    show.set_defaults(func=cmd_registry_show)

    validate = registry_sub.add_parser('validate', help='validate the registry file')
    validate.set_defaults(func=cmd_registry_validate)

    add = registry_sub.add_parser('add', help='add one source (configuration data only)')
    add.add_argument('--source-id', required=True)
    add.add_argument('--repository', required=True, help='owner/repo')
    add.add_argument('--evidence-scope', default='pull_request,issue,commit',
                     help='comma-separated subset of pull_request,issue,review,commit,ci_run')
    add.add_argument('--lookback-days', type=int, default=90)
    add.add_argument('--source-type', default='downstream_project',
                     choices=sources.SOURCE_TYPES)
    add.add_argument('--access-mode', default='public', choices=sources.ACCESS_MODES)
    add.add_argument('--include', action='append', help='inclusion filter kind:value (repeatable)')
    add.add_argument('--exclude', action='append', help='exclusion filter kind:value (repeatable)')
    add.add_argument('--notes', help='owner-approved applicability notes')
    add.add_argument('--disabled', action='store_true', help='register the source disabled')
    add.set_defaults(func=cmd_registry_add)

    disable = registry_sub.add_parser('disable', help='stop scanning one source')
    disable.add_argument('--source-id', required=True)
    disable.set_defaults(func=lambda args: cmd_registry_toggle(args, enable=False))

    enable = registry_sub.add_parser('enable', help='resume scanning one source')
    enable.add_argument('--source-id', required=True)
    enable.set_defaults(func=lambda args: cmd_registry_toggle(args, enable=True))

    remove = registry_sub.add_parser('remove', help='remove one source (lessons are kept)')
    remove.add_argument('--source-id', required=True)
    remove.set_defaults(func=cmd_registry_remove)

    scan = subparsers.add_parser('scan', help='run one scan and write a report')
    scan.add_argument('--offline', action='store_true',
                      help='do not collect; report existing records only')
    scan.add_argument('--source', action='append', help='restrict to a source_id (repeatable)')
    scan.add_argument('--analysis', help='JSON map evidence_url -> explicit lesson analysis')
    scan.add_argument('--max-pages', type=int, default=collect.DEFAULT_MAX_PAGES)
    scan.add_argument('--now', help='override the scan timestamp (RFC 3339, for tests)')
    scan.add_argument('--report-dir', default=str(DEFAULT_STATE_DIR / 'reports'))
    scan.add_argument('--dry-run', action='store_true',
                      help='write the report but not the registry / lesson store')
    scan.add_argument('--json', action='store_true')
    scan.set_defaults(func=cmd_scan)

    lesson = subparsers.add_parser('lesson', help='inspect or validate lesson records')
    lesson_sub = lesson.add_subparsers(dest='lesson_command', required=True)
    lesson_list = lesson_sub.add_parser('list', help='list stored lessons')
    lesson_list.add_argument('--json', action='store_true')
    lesson_list.set_defaults(func=cmd_lesson_list)
    lesson_validate = lesson_sub.add_parser('validate', help='validate lesson JSON file(s)')
    lesson_validate.add_argument('--file', required=True)
    lesson_validate.set_defaults(func=cmd_lesson_validate)
    return parser


def main(argv=None):
    parser = build_parser()
    args = parser.parse_args(argv)
    try:
        return args.func(args)
    except FileNotFoundError as exc:
        print(f'CONFIG ERROR: {exc}', file=sys.stderr)
        return EXIT_CONFIG
    except sources.RegistryError as exc:
        print(f'CONFIG ERROR: {exc}', file=sys.stderr)
        return EXIT_CONFIG
    except lessons.LessonError as exc:
        print(f'LESSON ERROR: {exc}', file=sys.stderr)
        return EXIT_CONFIG


if __name__ == '__main__':
    sys.exit(main())
