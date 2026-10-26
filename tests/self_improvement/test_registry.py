"""AC-1 / AC-2: the registry is configuration data, and cursors are honest.

AC-1 — adding, disabling and removing a source must be pure configuration edits.
Each test drives the data API (and the CLI in ``test_pipeline``) with a source the
code has never heard of; no branch is added anywhere.

AC-2 — the first scan uses the lookback window, an OK cursor becomes the next
start, a failed scan never moves the success watermark, and a backwards start
timestamp is refused instead of trusted.
"""
import json
import tempfile
import unittest
from pathlib import Path

from . import support
from agentorch_self_improvement import sources

REPO_ROOT = support.ROOT
DEFAULT_REGISTRY = REPO_ROOT / 'docs' / 'self-improvement' / 'sources.json'


class RegistryShapeTests(unittest.TestCase):
    # `docs/` is project configuration and is never packaged, so inside a release
    # archive this repository-only file legitimately does not exist. Same condition
    # and reason as tests/test_validate_kit.py.
    @unittest.skipUnless(DEFAULT_REGISTRY.is_file(),
                         'repository-only project config is not shipped')
    def test_default_registry_file_is_valid_and_declares_the_two_sources(self):
        registry = sources.Registry.load(DEFAULT_REGISTRY)
        self.assertEqual(registry.data['schema_version'], '1.0')
        ids = [entry['source_id'] for entry in registry.sources]
        self.assertEqual(ids, ['talkdnd', 'agentmgr'])
        repositories = {entry['source_id']: entry['repository'] for entry in registry.sources}
        self.assertEqual(repositories['talkdnd'], 'ctrlshiftaltdel/TalkDND')
        self.assertEqual(repositories['agentmgr'], 'ctrlshiftaltdel/AgentMgr')
        for entry in registry.sources:
            self.assertTrue(entry['enabled'])
            cursor = entry['scan_cursor']
            # The repository registry is no longer "fresh" once a real scan has
            # run, so a null cursor and a schema-valid cursor object are both
            # allowed here. The "fresh registry" rule keeps its own test below.
            if cursor is not None:
                self.assertEqual(sources.validate_cursor(cursor), [],
                                 f'{entry["source_id"]} cursor must be schema-valid')
                if cursor['last_scan_status'] == 'OK':
                    self.assertIsNotNone(cursor.get('last_successful_scan_at'),
                                         'an OK cursor must record a successful scan')
                    self.assertIsNotNone(cursor.get('last_scanned_commit'),
                                         'an OK cursor must record a commit watermark')
            self.assertEqual(entry['trust']['content_trust'], 'untrusted_data')
            self.assertFalse(entry['trust']['may_override_rules'])
            self.assertGreaterEqual(len(entry['evidence_scope']), 1)

    def test_fresh_registry_does_not_claim_a_scan(self):
        # The original invariant still applies to a freshly built registry, even
        # though the repository's own registry may now carry a real scan cursor.
        registry = sources.Registry(support.registry())
        for entry in registry.sources:
            self.assertIsNone(entry['scan_cursor'], 'a fresh registry must not claim a scan')

    def test_empty_sources_array_is_legal(self):
        self.assertEqual(sources.validate_registry(support.registry()), [])
        registry = sources.Registry({'schema_version': '1.0', 'sources': []})
        self.assertEqual(registry.sources, [])
        self.assertEqual(registry.enabled_sources(), [])

    def test_missing_required_field_is_rejected(self):
        entry = support.source()
        del entry['evidence_scope']
        errors = sources.validate_registry(support.registry(entry))
        self.assertIn('/sources/0/evidence_scope', {error['path'] for error in errors})

    def test_invalid_source_id_is_rejected(self):
        for bad in ('TalkDND', '1talkdnd', 'talkdnd_x', 'x'):
            with self.subTest(source_id=bad):
                errors = sources.validate_registry(support.registry(support.source(bad)))
                self.assertTrue(any(error['keyword'] == 'pattern' for error in errors))

    def test_duplicate_source_id_is_rejected(self):
        errors = sources.validate_registry(support.registry(support.source(),
                                                            support.source()))
        self.assertTrue(any(error['keyword'] == 'uniqueItems' for error in errors))

    def test_unknown_field_is_rejected(self):
        errors = sources.validate_registry(support.registry(support.source(wat='x')))
        self.assertTrue(any(error['keyword'] == 'additionalProperties' for error in errors))

    def test_ok_cursor_without_watermark_is_rejected(self):
        cursor = {'last_scan_status': 'OK', 'last_scan_started_at': support.T1}
        errors = sources.validate_registry(support.registry(support.source(scan_cursor=cursor)))
        paths = {error['path'] for error in errors}
        self.assertIn('/sources/0/scan_cursor/last_successful_scan_at', paths)
        self.assertIn('/sources/0/scan_cursor/last_scanned_commit', paths)

    def test_blocked_cursor_without_reason_is_rejected(self):
        cursor = {'last_scan_status': 'BLOCKED', 'last_scan_started_at': support.T1}
        errors = sources.validate_registry(support.registry(support.source(scan_cursor=cursor)))
        self.assertTrue(any(error['path'].endswith('/blocked_reason') for error in errors))

    def test_unsupported_schema_version_fails_closed_with_upgrade_hint(self):
        data = support.registry()
        data['schema_version'] = '2.0'
        errors = sources.validate_registry(data)
        self.assertTrue(any('unsupported schema_version' in error['message'] for error in errors))
        with self.assertRaises(sources.RegistryError):
            sources.Registry(data)

    def test_missing_registry_file_fails_closed_instead_of_defaulting(self):
        with tempfile.TemporaryDirectory() as tmp:
            missing = Path(tmp) / 'absent.json'
            with self.assertRaises(sources.RegistryError) as caught:
                sources.Registry.load(missing)
            self.assertIn('missing', str(caught.exception))

    def test_unparsable_registry_fails_closed(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / 'sources.json'
            path.write_text('{not json', encoding='utf-8')
            with self.assertRaises(sources.RegistryError) as caught:
                sources.Registry.load(path)
            self.assertIn('not valid JSON', str(caught.exception))

    def test_registry_strings_may_not_carry_credentials_or_home_paths(self):
        for value in (support.FAKE_TOKEN, '/home/someone/private', 'C:\\Users\\someone\\x'):
            with self.subTest(value=value):
                errors = sources.validate_registry(support.registry(
                    support.source(owner_approved_notes=value)))
                self.assertTrue(any(error['keyword'] == 'privacy' for error in errors))

    def test_repository_must_be_bare_owner_repo(self):
        for value in ('https://github.com/owner/repo', 'git@github.com:owner/repo',
                      '--upload-pack=evil', 'owner/repo/tree/main'):
            with self.subTest(value=value):
                errors = sources.validate_registry(support.registry(
                    support.source(repository=value)))
                self.assertTrue(errors)

    def test_evidence_scope_must_be_a_non_empty_known_subset(self):
        for value in ([], ['pull_request', 'pull_request'], ['wiki']):
            with self.subTest(value=value):
                errors = sources.validate_registry(support.registry(
                    support.source(evidence_scope=value)))
                self.assertTrue(errors)


class RegistryMutationTests(unittest.TestCase):
    def test_add_source_is_pure_configuration(self):
        registry = sources.Registry(support.registry())
        new = sources.make_source('exampleproj', 'owner/example',
                                  evidence_scope=('pull_request', 'commit'),
                                  lookback_days=14)
        updated = sources.add_source(registry, new)
        self.assertEqual([entry['source_id'] for entry in updated.sources],
                         ['talkdnd', 'exampleproj'])
        self.assertEqual(updated.sources[-1]['repository'], 'owner/example')
        self.assertIsNone(updated.sources[-1]['scan_cursor'])
        # The original registry is untouched.
        self.assertEqual(len(registry.sources), 1)

    def test_adding_an_unknown_repository_does_not_need_code(self):
        registry = sources.Registry(support.registry(support.source()))
        for index, repository in enumerate(('acme/one', 'acme/two', 'acme/three')):
            registry = sources.add_source(
                registry,
                sources.make_source(f'acme-{index}', repository,
                                    evidence_scope=('issue',)))
        self.assertEqual(len(registry.enabled_sources()), 4)
        self.assertEqual(sources.validate_registry(registry.to_dict()), [])

    def test_repointing_an_existing_source_id_is_rejected(self):
        registry = sources.Registry(support.registry())
        with self.assertRaises(sources.RegistryError) as caught:
            sources.add_source(registry, sources.make_source('talkdnd', 'other/repo'))
        self.assertIn('re-pointing', str(caught.exception))

    def test_disable_keeps_the_entry_and_stops_scanning(self):
        registry = sources.Registry(support.registry())
        updated = sources.disable_source(registry, 'talkdnd')
        self.assertEqual(len(updated.sources), 1)
        self.assertFalse(updated.get('talkdnd')['enabled'])
        self.assertEqual(updated.enabled_sources(), [])
        reenabled = sources.enable_source(updated, 'talkdnd')
        self.assertEqual([entry['source_id'] for entry in reenabled.enabled_sources()],
                         ['talkdnd'])

    def test_remove_can_empty_the_registry_and_never_touches_lessons(self):
        registry = sources.Registry(support.registry())
        registry = sources.remove_source(registry, 'talkdnd')
        self.assertEqual(registry.sources, [])
        self.assertEqual(sources.validate_registry(registry.to_dict()), [])
        with self.assertRaises(sources.RegistryError):
            sources.remove_source(registry, 'talkdnd')

    def test_save_and_reload_roundtrip(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / 'nested' / 'sources.json'
            registry = sources.add_source(
                sources.Registry(support.registry()),
                sources.make_source('acme', 'acme/app', evidence_scope=('issue',)))
            registry.save(path)
            reloaded = sources.Registry.load(path)
            self.assertEqual([entry['source_id'] for entry in reloaded.sources],
                             ['talkdnd', 'acme'])
            self.assertEqual(json.loads(path.read_text(encoding='utf-8'))['schema_version'], '1.0')


class CursorTests(unittest.TestCase):
    def test_first_scan_uses_the_lookback_window(self):
        window = sources.scan_window(support.source(), sources.parse_timestamp(support.T2))
        self.assertEqual(window['mode'], 'first_scan')
        self.assertEqual(window['since'], '2026-07-22T00:00:00Z')  # 90 days before T2
        self.assertEqual(window['until'], support.T2)

    def test_incremental_scan_starts_at_the_last_success(self):
        cursor = {'last_scan_status': 'OK', 'last_scan_started_at': support.T1,
                  'last_successful_scan_at': '2026-09-02T00:00:00Z',
                  'last_scanned_commit': support.SHA_A, 'blocked_reason': None}
        window = sources.scan_window(support.source(scan_cursor=cursor),
                                     sources.parse_timestamp(support.T2))
        self.assertEqual(window['mode'], 'incremental')
        self.assertEqual(window['since'], '2026-09-02T00:00:00Z')

    def test_failed_scan_retries_from_the_last_success(self):
        cursor = {'last_scan_status': 'BLOCKED', 'last_scan_started_at': support.T1,
                  'last_successful_scan_at': '2026-08-15T00:00:00Z',
                  'last_scanned_commit': support.SHA_A, 'blocked_reason': 'rate_limited'}
        window = sources.scan_window(support.source(scan_cursor=cursor),
                                     sources.parse_timestamp(support.T2))
        self.assertEqual(window['mode'], 'retry')
        self.assertEqual(window['since'], '2026-08-15T00:00:00Z')

    def test_failed_first_scan_falls_back_to_lookback(self):
        cursor = {'last_scan_status': 'UNKNOWN', 'last_scan_started_at': support.T1,
                  'last_successful_scan_at': None, 'last_scanned_commit': None,
                  'blocked_reason': 'timeout'}
        window = sources.scan_window(support.source(scan_cursor=cursor),
                                     sources.parse_timestamp(support.T2))
        self.assertEqual(window['mode'], 'lookback_after_failed_scan')

    def test_only_ok_advances_the_success_watermark(self):
        previous = {'last_scan_status': 'OK', 'last_scan_started_at': support.T0,
                    'last_successful_scan_at': support.T1,
                    'last_scanned_commit': support.SHA_A, 'blocked_reason': None}
        advanced = sources.advance_cursor(previous, status='OK', started_at=support.T2,
                                          finished_at=support.T2,
                                          scanned_commit=support.SHA_B)
        self.assertEqual(advanced['last_scanned_commit'], support.SHA_B)
        self.assertEqual(advanced['last_successful_scan_at'], support.T2)
        for status, reason in (('PARTIAL', None), ('BLOCKED', 'forbidden'),
                               ('UNKNOWN', 'malformed_response')):
            with self.subTest(status=status):
                cursor = sources.advance_cursor(previous, status=status,
                                                started_at=support.T2,
                                                finished_at=support.T2,
                                                blocked_reason=reason)
                self.assertEqual(cursor['last_scanned_commit'], support.SHA_A)
                self.assertEqual(cursor['last_successful_scan_at'], support.T1)
                self.assertEqual(cursor['last_scan_status'], status)

    def test_ok_requires_a_commit_watermark(self):
        with self.assertRaises(ValueError):
            sources.advance_cursor(None, status='OK', started_at=support.T1)

    def test_blocked_requires_a_reason(self):
        with self.assertRaises(ValueError):
            sources.advance_cursor(None, status='BLOCKED', started_at=support.T1)

    def test_backwards_start_is_a_cursor_regression(self):
        previous = {'last_scan_status': 'OK', 'last_scan_started_at': support.T2,
                    'last_successful_scan_at': support.T2,
                    'last_scanned_commit': support.SHA_A, 'blocked_reason': None}
        with self.assertRaises(sources.CursorRegression):
            sources.advance_cursor(previous, status='OK', started_at=support.T1,
                                   scanned_commit=support.SHA_B)


if __name__ == '__main__':
    unittest.main()
