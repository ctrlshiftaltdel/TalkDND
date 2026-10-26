"""Tests for the release manifest and the read-only drift diagnosis.

`scripts/check_drift.py` owns both the single classification rule and the
manifest generator; these tests exercise them directly (no second copy of the
rules) and prove that a diagnosis never touches the inspected tree.
"""
import importlib.util
import json
import os
import re
import subprocess
import sys
import tempfile
import unittest
import zipfile
from datetime import date
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SCRIPT = ROOT / 'scripts' / 'check_drift.py'
VALIDATOR = ROOT / 'scripts' / 'validate_kit.py'
BUILD = ROOT / 'scripts' / 'build_release.py'
VERSIONS = ROOT / 'scripts' / 'update_agentorch' / 'versions.py'


def load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


drift = load('check_drift_under_test', SCRIPT)
validate = load('validate_kit_under_test', VALIDATOR)

SUITE = 'Agent-Orchestration Starter Kit'
FORMAT = '1.2'

TREE_FILES = {
    'kit/orchestration.md': '# core contract\n',
    'editions/local/master.md': '# local edition\n',
    'docs/PROJECT.md': '- `edition`: `local`\n',
}


class FixtureTree:
    """A tiny project tree with a valid manifest; used by the diagnosis tests."""

    def __init__(self, tmp, files=None):
        self.root = Path(tmp)
        self.files = dict(TREE_FILES if files is None else files)
        for rel, text in self.files.items():
            self.write(rel, text)

    def write(self, rel, text):
        path = self.root / rel
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text, encoding='utf-8')

    def packed(self):
        return {rel: (self.root / rel).read_bytes() for rel in self.manifest_paths()}

    def manifest_paths(self):
        return sorted(rel for rel in self.files if rel != drift.MANIFEST_NAME)

    def emit_manifest(self, **overrides):
        manifest = drift.build_manifest(self.packed(), SUITE, FORMAT)
        manifest.update(overrides)
        (self.root / drift.MANIFEST_NAME).write_text(
            drift.dumps_manifest(manifest), encoding='utf-8')
        return manifest

    def remove_manifest(self):
        (self.root / drift.MANIFEST_NAME).unlink()


def snapshot(root):
    """Every file's bytes + mtime_ns, keyed by relative path (read-only probe)."""
    state = {}
    for path in sorted(Path(root).rglob('*')):
        if path.is_file():
            stat = path.stat()
            state[path.relative_to(root).as_posix()] = (
                path.read_bytes(), stat.st_mtime_ns, stat.st_size)
    return state


def run_cli(*args, cwd=None):
    env = dict(os.environ, PYTHONDONTWRITEBYTECODE='1')
    return subprocess.run([sys.executable, '-B', str(SCRIPT), *args],
                          capture_output=True, text=True, cwd=cwd, env=env)


class ManifestGenerationTests(unittest.TestCase):
    def test_entries_are_sorted_and_carry_the_contract_fields(self):
        files = {'editions/local/master.md': b'e', 'kit/orchestration.md': b'c',
                 'templates/plan.md': b't', 'docs/PROJECT.md': b'p'}
        manifest = drift.build_manifest(files, SUITE, FORMAT)
        self.assertEqual(sorted(manifest), ['files', 'format_version', 'schema_version', 'suite'])
        self.assertEqual(manifest['schema_version'], drift.MANIFEST_SCHEMA_VERSION)
        self.assertEqual(manifest['suite'], SUITE)
        self.assertEqual(manifest['format_version'], FORMAT)
        paths = [entry['path'] for entry in manifest['files']]
        self.assertEqual(paths, sorted(paths))
        for entry in manifest['files']:
            self.assertEqual(sorted(entry), ['category', 'path', 'sha256', 'size'])
            self.assertEqual(entry['size'], len(files[entry['path']]))
            self.assertRegex(entry['sha256'], r'^[0-9a-f]{64}$')

    def test_manifest_has_no_timestamp_and_is_byte_stable(self):
        files = {'kit/orchestration.md': b'# core\n', 'kit/orchestration.md': b'# boot\n'}
        first = drift.dumps_manifest(drift.build_manifest(files, SUITE, FORMAT))
        second = drift.dumps_manifest(drift.build_manifest(files, SUITE, FORMAT))
        self.assertEqual(first, second)
        payload = json.loads(first)
        for key in drift.FORBIDDEN_MANIFEST_KEYS:
            self.assertNotIn(key, payload)

    def test_hash_and_size_follow_lf_normalization(self):
        manifest = drift.build_manifest({'kit/orchestration.md': b'# a\r\n# b\r\n'},
                                        SUITE, FORMAT)
        entry = manifest['files'][0]
        self.assertEqual(entry['size'], len(b'# a\n# b\n'))
        self.assertEqual(entry['sha256'], drift.sha256_bytes(b'# a\n# b\n'))

    def test_binary_payload_is_hashed_verbatim(self):
        payload = b'\x00\x01binary'
        entry = drift.build_manifest({'LICENSE.md': payload}, SUITE, FORMAT)['files'][0]
        self.assertEqual(entry['sha256'], drift.sha256_bytes(payload))
        self.assertEqual(entry['size'], len(payload))

    def test_category_rule_is_single_and_complete(self):
        cases = {
            'AGENTS.md': 'core',
            'scripts/check_drift.py': 'core',
            'editions/github/master.md': 'edition',
            'templates/task-packet.md': 'template',
            'docs/PROJECT.md': 'project-profile',
            'PLAN.md': 'project-profile',
        }
        for path, expected in cases.items():
            with self.subTest(path=path):
                self.assertEqual(drift.classify_upstream(path), expected)

    def test_local_only_classification_separates_extensions_from_unknown(self):
        self.assertEqual(drift.classify_local('src/app.py'), 'project-extension')
        self.assertEqual(drift.classify_local('docs/design.md'), 'project-extension')
        self.assertEqual(drift.classify_local('PLAN.md'), 'project-profile')       # legacy root (<=1.3)
        self.assertEqual(drift.classify_local('docs/PLAN.md'), 'project-profile')  # current location (1.4)
        self.assertEqual(drift.classify_local('editions/local/EXTRA.md'), 'unknown')
        self.assertEqual(drift.classify_local('templates/EXTRA.md'), 'unknown')
        self.assertEqual(drift.classify_local('tests/test_project.py'), 'unknown')

    @unittest.skipUnless(BUILD.is_file(), 'build_release.py is repository-only')
    def test_two_archives_of_one_tree_are_byte_identical(self):
        builder = load('build_release_under_test', BUILD)
        files = {
            'kit/orchestration.md': b'# core\n',
            'editions/local/master.md': b'# edition\n',
            'docs/PROJECT.md': b'# project\n',
        }
        manifest = drift.build_manifest(files, SUITE, FORMAT)
        files[drift.MANIFEST_NAME] = drift.dumps_manifest(manifest).encode('utf-8')
        with tempfile.TemporaryDirectory() as tmp:
            first = Path(tmp) / 'a.zip'
            second = Path(tmp) / 'b.zip'
            builder.write_zip(first, files, 'agentorch-20260101')
            builder.write_zip(second, files, 'agentorch-20260101')
            self.assertEqual(drift.sha256_bytes(first.read_bytes()),
                             drift.sha256_bytes(second.read_bytes()))
            with zipfile.ZipFile(first) as archive:
                self.assertEqual(sorted(archive.namelist()), sorted(
                    f'agentorch-20260101/{rel}' for rel in files))


@unittest.skipUnless(BUILD.is_file(), 'build_release.py is repository-only')
class ReleaseAssetNamingTests(unittest.TestCase):
    """The public release name is a naming choice, not a format change.

    `--release-version 1.6` must produce `AgentOrch-1.6.zip` whose single
    top-level directory is `AgentOrch-1.6/`; the default date naming has to keep
    working because CI and the baseline-rebuild instructions still use it.
    """

    @classmethod
    def setUpClass(cls):
        cls.builder = load('build_release_naming_under_test', BUILD)

    def test_release_version_names_the_asset_and_top_dir(self):
        stem = self.builder.release_asset_stem(None, '1.6')
        self.assertEqual(stem, 'AgentOrch-1.6')
        self.assertEqual(f'{stem}.zip', 'AgentOrch-1.6.zip')
        self.assertEqual(f'{stem}/{drift.MANIFEST_NAME}', 'AgentOrch-1.6/MANIFEST.json')

    def test_patch_release_version_is_allowed(self):
        self.assertEqual(self.builder.release_asset_stem(None, '1.6.1'), 'AgentOrch-1.6.1')

    def test_default_date_naming_still_works(self):
        self.assertEqual(self.builder.release_asset_stem('20261026'), 'agentorch-20261026')
        today = date.today().strftime('%Y%m%d')
        self.assertEqual(self.builder.release_asset_stem(), f'agentorch-{today}')

    def test_release_and_date_naming_never_combine(self):
        with self.assertRaises(SystemExit):
            self.builder.release_asset_stem('20261026', '1.6')

    def test_invalid_release_versions_are_rejected(self):
        for bad in ('v1.6', '1', '1.6.0.1', '1.6-rc1', 'latest', ''):
            with self.subTest(version=bad):
                with self.assertRaises(SystemExit):
                    self.builder.release_asset_stem(None, bad)

    def test_invalid_date_stamps_are_rejected(self):
        for bad in ('2026-10-26', '2026102', '202610266'):
            with self.subTest(stamp=bad):
                with self.assertRaises(SystemExit):
                    self.builder.release_asset_stem(bad)

    def test_archive_follows_the_release_stem_and_keeps_the_format_version(self):
        files = {
            'kit/orchestration.md': b'# core\n',
            'editions/local/master.md': b'# edition\n',
        }
        manifest = drift.build_manifest(files, SUITE, FORMAT)
        self.assertEqual(manifest['format_version'], FORMAT)
        files[drift.MANIFEST_NAME] = drift.dumps_manifest(manifest).encode('utf-8')
        with tempfile.TemporaryDirectory() as tmp:
            first = Path(tmp) / 'AgentOrch-1.6.zip'
            second = Path(tmp) / 'second.zip'
            self.builder.write_zip(first, files, 'AgentOrch-1.6')
            self.builder.write_zip(second, files, 'AgentOrch-1.6')
            self.assertEqual(drift.sha256_bytes(first.read_bytes()),
                             drift.sha256_bytes(second.read_bytes()))
            with zipfile.ZipFile(first) as archive:
                names = sorted(archive.namelist())
                shipped = json.loads(archive.read(f'AgentOrch-1.6/{drift.MANIFEST_NAME}'))
            self.assertEqual(names, sorted(f'AgentOrch-1.6/{rel}' for rel in files))
            self.assertIn('AgentOrch-1.6/MANIFEST.json', names)
            self.assertEqual(shipped['format_version'], FORMAT)

    def test_cli_rejects_a_v_prefixed_version_without_building(self):
        with tempfile.TemporaryDirectory() as tmp:
            result = subprocess.run(
                [sys.executable, '-B', str(BUILD), '--release-version', 'v1.6',
                 '--output-dir', tmp], capture_output=True, text=True)
            self.assertEqual(result.returncode, 1)
            self.assertIn('--release-version', result.stderr)
            self.assertEqual(list(Path(tmp).iterdir()), [])

    def test_cli_rejects_date_and_release_version_together(self):
        with tempfile.TemporaryDirectory() as tmp:
            result = subprocess.run(
                [sys.executable, '-B', str(BUILD), '--date', '20261026',
                 '--release-version', '1.6', '--output-dir', tmp],
                capture_output=True, text=True)
            self.assertEqual(result.returncode, 2)
            self.assertIn('not allowed with argument', result.stderr)
            self.assertEqual(list(Path(tmp).iterdir()), [])


class VersionConsistencyTests(unittest.TestCase):
    """Format and Release version have to agree for the published tag.

    The update engine's VERSION_MATCH check accepts a release only when the
    tag's major.minor equals MANIFEST.json's format_version (versions.py owns
    that rule), so the two numbers in VERSION.md cannot be reviewed separately:
    a `v1.6` tag with a `1.5` Format version is rejected as UNSAFE_PACKAGE.
    """

    @classmethod
    def setUpClass(cls):
        cls.versions = load('versions_under_test', VERSIONS)

    def release_version(self):
        text = (ROOT / 'VERSION.md').read_text(encoding='utf-8')
        match = re.search(r'^- Release version:\s*(\S+)', text, re.M)
        self.assertIsNotNone(match, 'VERSION.md must declare "Release version: <x.y>"')
        return match.group(1)

    def test_version_file_parses_as_suite_and_format_1_6(self):
        self.assertEqual(drift.read_version(ROOT), (SUITE, '1.6'))

    def test_release_tag_matches_the_format_version(self):
        release = self.release_version()
        _, format_version = drift.read_version(ROOT)
        self.assertEqual(release, '1.6')
        self.assertEqual(format_version, '1.6')
        self.assertTrue(self.versions.tag_matches_format(f'v{release}', format_version))
        # The previous tag must NOT be accepted against the current Format: the
        # rule compares major.minor, so a silent mismatch would be a real defect.
        self.assertFalse(self.versions.tag_matches_format('v1.5', format_version))

    def test_manifest_generated_from_version_file_records_format_1_6(self):
        suite, format_version = drift.read_version(ROOT)
        manifest = drift.build_manifest({'AGENTS.md': b'# agents\n'}, suite, format_version)
        self.assertEqual(manifest['format_version'], '1.6')
        shipped = json.loads(drift.dumps_manifest(manifest))
        self.assertEqual(shipped['format_version'], '1.6')
        self.assertEqual(shipped['suite'], SUITE)


class ManifestValidationTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.tree = FixtureTree(self.tmp.name)
        self.manifest = self.tree.emit_manifest()

    def tearDown(self):
        self.tmp.cleanup()

    def test_valid_manifest_passes(self):
        self.assertEqual(drift.validate_manifest(self.tree.root, self.manifest), [])

    def test_manifest_built_from_the_kit_passes(self):
        required = [rel for rel in validate.REQUIRED]
        files = {rel: (ROOT / rel).read_bytes() for rel in required}
        manifest = drift.build_manifest(files, SUITE, FORMAT)
        self.assertEqual(drift.manifest_structure_errors(manifest), [])
        self.assertEqual(drift.validate_manifest(ROOT, manifest), [])
        self.assertEqual(len(manifest['files']), len(required))

    def test_missing_file_is_reported(self):
        (self.tree.root / 'editions/local/master.md').unlink()
        errors = drift.validate_manifest(self.tree.root, self.manifest)
        self.assertTrue(any('missing on disk' in e for e in errors), errors)

    def test_hash_mismatch_is_reported(self):
        self.tree.write('kit/orchestration.md', '# tampered\n')
        errors = drift.validate_manifest(self.tree.root, self.manifest)
        self.assertTrue(any('sha256 does not match' in e for e in errors), errors)

    def test_size_mismatch_is_reported(self):
        manifest = json.loads(drift.dumps_manifest(self.manifest))
        manifest['files'][0]['size'] += 1
        errors = drift.validate_manifest(self.tree.root, manifest)
        self.assertTrue(any('size does not match' in e for e in errors), errors)

    def test_illegal_category_is_reported(self):
        manifest = json.loads(drift.dumps_manifest(self.manifest))
        manifest['files'][0]['category'] = 'vendor'
        errors = drift.manifest_structure_errors(manifest)
        self.assertTrue(any('unknown category' in e for e in errors), errors)

    def test_category_that_breaks_the_single_rule_is_reported(self):
        manifest = json.loads(drift.dumps_manifest(self.manifest))
        manifest['files'][0]['category'] = 'template'
        errors = drift.manifest_structure_errors(manifest)
        self.assertTrue(any('does not match the single classification rule' in e for e in errors), errors)

    def test_timestamp_is_rejected(self):
        manifest = json.loads(drift.dumps_manifest(self.manifest))
        manifest['generated_at'] = '2026-10-22T00:00:00Z'
        errors = drift.manifest_structure_errors(manifest)
        self.assertTrue(any('must not carry' in e for e in errors), errors)

    def test_unsorted_and_duplicate_paths_are_rejected(self):
        manifest = json.loads(drift.dumps_manifest(self.manifest))
        manifest['files'] = list(reversed(manifest['files']))
        errors = drift.manifest_structure_errors(manifest)
        self.assertTrue(any('not sorted' in e for e in errors), errors)
        manifest['files'] = [manifest['files'][0], manifest['files'][0]]
        errors = drift.manifest_structure_errors(manifest)
        self.assertTrue(any('duplicates path' in e for e in errors), errors)

    def test_path_escape_is_rejected(self):
        manifest = json.loads(drift.dumps_manifest(self.manifest))
        manifest['files'][0]['path'] = '../AGENTS.md'
        errors = drift.manifest_structure_errors(manifest)
        self.assertTrue(any('must not contain' in e for e in errors), errors)

    def test_manifest_does_not_list_itself(self):
        files = {'kit/orchestration.md': b'c', drift.MANIFEST_NAME: b'{}'}
        manifest = drift.build_manifest(files, SUITE, FORMAT)
        self.assertEqual([entry['path'] for entry in manifest['files']],
                         ['kit/orchestration.md'])

    def test_schema_newer_than_supported_is_not_a_structure_error(self):
        manifest = json.loads(drift.dumps_manifest(self.manifest))
        manifest['schema_version'] = max(drift.SUPPORTED_MANIFEST_SCHEMAS) + 1
        self.assertEqual(drift.validate_manifest(self.tree.root, manifest), [])


class DiagnosisTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.tree = FixtureTree(self.tmp.name)
        self.tree.emit_manifest()

    def tearDown(self):
        self.tmp.cleanup()

    def paths(self, report, key):
        return [item['path'] for item in report['findings'][key]]

    def test_clean_tree(self):
        report = drift.diagnose(self.tree.root)
        self.assertEqual(report['status'], 'clean')
        self.assertEqual(report['exit_code'], 0)
        self.assertEqual(report['summary']['checked'], len(TREE_FILES))
        for key in ('missing', 'modified', 'unknown', 'extensions'):
            self.assertEqual(report['findings'][key], [])

    def test_modified_core_file_is_drift(self):
        self.tree.write('kit/orchestration.md', '# changed contract\n')
        report = drift.diagnose(self.tree.root)
        self.assertEqual(report['status'], 'drift')
        self.assertEqual(report['exit_code'], 2)
        self.assertEqual(self.paths(report, 'modified'), ['kit/orchestration.md'])
        item = report['findings']['modified'][0]
        self.assertEqual(item['category'], 'core')
        self.assertNotEqual(item['expected_sha256'], item['actual_sha256'])

    def test_missing_core_file_is_drift(self):
        (self.tree.root / 'editions/local/master.md').unlink()
        report = drift.diagnose(self.tree.root)
        self.assertEqual(report['exit_code'], 2)
        self.assertEqual(self.paths(report, 'missing'), ['editions/local/master.md'])

    def test_project_extension_is_reported_but_not_drift(self):
        self.tree.write('src/app.py', 'print("project code")\n')
        self.tree.write('docs/notes.md', '# notes\n')
        report = drift.diagnose(self.tree.root)
        self.assertEqual(report['status'], 'clean')
        self.assertEqual(report['exit_code'], 0)
        self.assertEqual(sorted(self.paths(report, 'extensions')),
                         ['docs/notes.md', 'src/app.py'])
        self.assertEqual(report['findings']['unknown'], [])
        self.assertTrue(any('extensions' in note for note in report['notes']))

    def test_extra_file_in_an_upstream_directory_is_unknown_and_drift(self):
        self.tree.write('editions/local/EXTRA.md', '# leftover from another version\n')
        report = drift.diagnose(self.tree.root)
        self.assertEqual(report['exit_code'], 2)
        self.assertEqual(self.paths(report, 'unknown'), ['editions/local/EXTRA.md'])
        self.assertNotIn('editions/local/EXTRA.md', self.paths(report, 'extensions'))

    def test_project_profile_edit_is_not_drift(self):
        self.tree.write('docs/PROJECT.md', '- `edition`: `github`\n')
        self.tree.write('PLAN.md', '# design goals\n')
        report = drift.diagnose(self.tree.root)
        self.assertEqual(report['status'], 'clean')
        self.assertEqual(sorted(self.paths(report, 'project_profile')),
                         ['PLAN.md', 'docs/PROJECT.md'])
        self.assertEqual(report['findings']['modified'], [])

    def test_tree_without_manifest_reports_unknown_version(self):
        self.tree.remove_manifest()
        report = drift.diagnose(self.tree.root)
        self.assertEqual(report['status'], 'version-unknown')
        self.assertEqual(report['exit_code'], 2)
        self.assertFalse(report['manifest']['present'])
        self.assertIsNone(report['version_span']['supported'])
        self.assertEqual(report['findings'], drift._empty_findings())
        self.assertTrue(any('baseline' in note for note in report['notes']))
        self.assertTrue(any('never guesses' in note for note in report['notes']))

    def test_newer_manifest_schema_is_reported_as_drift(self):
        self.tree.emit_manifest(schema_version=max(drift.SUPPORTED_MANIFEST_SCHEMAS) + 1)
        report = drift.diagnose(self.tree.root)
        self.assertEqual(report['status'], 'drift')
        self.assertFalse(report['version_span']['supported'])
        self.assertTrue(any('outside this tool' in note for note in report['notes']))

    def test_crlf_checkout_is_not_reported_as_modified(self):
        path = self.tree.root / 'kit/orchestration.md'
        path.write_bytes(b'# core contract\r\n')
        report = drift.diagnose(self.tree.root)
        self.assertEqual(report['findings']['modified'], [])

    def test_diagnosis_writes_nothing(self):
        self.tree.write('src/app.py', 'x = 1\n')
        self.tree.write('editions/local/EXTRA.md', 'extra\n')
        before = snapshot(self.tree.root)
        drift.diagnose(self.tree.root)
        result = run_cli(str(self.tree.root), cwd=str(self.tree.root))
        self.assertIn(result.returncode, (0, 2), result.stderr)
        self.assertEqual(snapshot(self.tree.root), before)

    def test_invalid_root_is_a_tool_error(self):
        missing = self.tree.root / 'does-not-exist'
        result = run_cli(str(missing))
        self.assertEqual(result.returncode, 1)
        self.assertIn('ERROR', result.stderr)
        self.assertIn('does not exist', result.stderr)

    def test_file_as_root_is_a_tool_error(self):
        result = run_cli(str(self.tree.root / 'kit/orchestration.md'))
        self.assertEqual(result.returncode, 1)
        self.assertIn('not a directory', result.stderr)

    def test_malformed_manifest_is_a_tool_error(self):
        (self.tree.root / drift.MANIFEST_NAME).write_text('{not json', encoding='utf-8')
        result = run_cli(str(self.tree.root))
        self.assertEqual(result.returncode, 1)
        self.assertIn('not valid JSON', result.stderr)

    def test_unsorted_manifest_is_a_tool_error(self):
        manifest = json.loads(drift.dumps_manifest(drift.build_manifest(
            self.tree.packed(), SUITE, FORMAT)))
        manifest['files'] = list(reversed(manifest['files']))
        (self.tree.root / drift.MANIFEST_NAME).write_text(
            json.dumps(manifest), encoding='utf-8')
        result = run_cli(str(self.tree.root))
        self.assertEqual(result.returncode, 1)
        self.assertIn('is invalid', result.stderr)


class CliContractTests(unittest.TestCase):
    REPORT_KEYS = {'exit_code', 'findings', 'manifest', 'notes', 'report_schema_version',
                   'root', 'status', 'summary', 'tool', 'version_span'}

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.tree = FixtureTree(self.tmp.name)
        self.tree.emit_manifest()

    def tearDown(self):
        self.tmp.cleanup()

    def test_json_mode_prints_only_json_on_stdout(self):
        self.tree.write('kit/orchestration.md', '# changed\n')
        result = run_cli(str(self.tree.root), '--json')
        self.assertEqual(result.returncode, 2, result.stderr)
        report = json.loads(result.stdout)
        self.assertEqual(report['status'], 'drift')
        self.assertEqual(report['exit_code'], 2)
        self.assertEqual(report['report_schema_version'], drift.REPORT_SCHEMA_VERSION)
        self.assertEqual(set(report), self.REPORT_KEYS)
        self.assertEqual(report['findings']['modified'][0]['path'], 'kit/orchestration.md')
        self.assertIn('status: drift', result.stderr)

    def test_json_mode_reports_clean_with_exit_zero(self):
        result = run_cli(str(self.tree.root), '--json')
        self.assertEqual(result.returncode, 0, result.stderr)
        report = json.loads(result.stdout)
        self.assertEqual(report['status'], 'clean')
        self.assertEqual(report['exit_code'], 0)

    def test_json_error_keeps_the_same_top_level_keys(self):
        result = run_cli(str(self.tree.root / 'nope'), '--json')
        self.assertEqual(result.returncode, 1)
        report = json.loads(result.stdout)
        self.assertEqual(set(report), self.REPORT_KEYS | {'error'})
        self.assertEqual(report['status'], 'error')
        self.assertEqual(report['exit_code'], 1)

    def test_human_mode_prints_a_summary(self):
        self.tree.write('src/app.py', 'x = 1\n')
        result = run_cli(str(self.tree.root))
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn('status: clean', result.stdout)
        self.assertIn('[extension] src/app.py', result.stdout)
        self.assertEqual(result.stderr, '')


class ValidateKitManifestTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.root = Path(self.tmp.name)

    def tearDown(self):
        self.tmp.cleanup()

    def test_repository_without_manifest_skips_the_check(self):
        self.assertEqual(validate.check_release_manifest(ROOT), [])

    def test_present_manifest_with_wrong_hash_is_reported(self):
        (self.root / 'AGENTS.md').write_text('# agents\n', encoding='utf-8')
        manifest = drift.build_manifest({'AGENTS.md': b'# other\n'}, SUITE, FORMAT)
        (self.root / drift.MANIFEST_NAME).write_text(
            drift.dumps_manifest(manifest), encoding='utf-8')
        errors = validate.check_release_manifest(self.root)
        self.assertTrue(any('sha256 does not match' in e for e in errors), errors)
        self.assertTrue(any('does not list required file' in e for e in errors), errors)

    def test_malformed_manifest_is_reported(self):
        (self.root / drift.MANIFEST_NAME).write_text('nope', encoding='utf-8')
        errors = validate.check_release_manifest(self.root)
        self.assertTrue(any('not valid JSON' in e for e in errors), errors)

    def test_manifest_is_not_part_of_the_required_list(self):
        self.assertNotIn(drift.MANIFEST_NAME, validate.REQUIRED)
        self.assertIn('scripts/check_drift.py', validate.REQUIRED)
        self.assertIn('tests/test_release_drift.py', validate.REQUIRED)


if __name__ == '__main__':
    unittest.main()
