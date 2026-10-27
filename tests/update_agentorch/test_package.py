"""Package verification tests: every §3 check has at least one negative fixture.

All archives are synthesized in memory; nothing is downloaded and nothing is
extracted outside the engine's own temporary directory.
"""
from __future__ import annotations

import io
import os
import unittest
import zipfile

from . import support

package = support.package


UPSTREAM = {
    'AGENTS.md': b'# agents\n',
    'kit/orchestration.md': b'# orchestration\n',
    'templates/task-packet.md': b'# packet\n',
}


def results(result) -> dict[str, str]:
    return {entry['check']: entry['result'] for entry in result.checks}


class PackageVerificationTests(unittest.TestCase):
    def verify(self, files=None, manifest=None, *, tag='v1.6', digest='auto',
               manifest_format=None, expected_suite=None,
               limits=package.DEFAULT_LIMITS, **zip_kwargs):
        files = dict(UPSTREAM if files is None else files)
        if manifest is None:
            format_version = manifest_format or '.'.join(tag.lstrip('v').split('.')[:2])
            manifest = support.build_manifest(files, format_version=format_version)
        payload = support.make_zip(files, manifest, **zip_kwargs)
        release = support.release_info(tag, payload, digest=digest)
        return package.verify_package(release, payload, expected_suite=expected_suite,
                                      limits=limits), manifest, payload

    def test_clean_package_passes_every_check(self):
        result, manifest, payload = self.verify()
        self.assertTrue(result.ok)
        self.assertEqual(result.status, 'VERIFIED')
        self.assertEqual([entry['check'] for entry in result.checks], list(package.CHECK_ORDER))
        self.assertEqual(set(results(result).values()), {'PASS'})
        self.assertEqual(result.manifest, manifest)
        self.assertEqual(result.files['AGENTS.md'], b'# agents\n')
        self.assertEqual(result.version, '1.6.0')
        self.assertEqual(result.format_version, '1.6')
        self.assertEqual(result.suite, support.SUITE)

    def test_check_order_is_the_frozen_order(self):
        self.assertEqual(package.CHECK_ORDER, (
            'SOURCE_ALLOWLIST', 'RELEASE_CHANNEL', 'ASSET_SHA256', 'MANIFEST_SCHEMA',
            'MANIFEST_SHA256', 'MEMBER_PATHS', 'SYMLINKS', 'FILE_SET',
            'EXPECTED_LAYOUT', 'VERSION_MATCH',
        ))

    def test_asset_digest_mismatch_is_unsafe(self):
        result, _, _ = self.verify(digest='sha256:' + '0' * 64)
        self.assertFalse(result.ok)
        self.assertEqual(result.status, 'UNSAFE_PACKAGE')
        self.assertEqual(results(result)['ASSET_SHA256'], 'FAIL')
        self.assertEqual(result.failed_check, 'ASSET_SHA256')

    def test_missing_asset_digest_is_unsafe(self):
        result, _, _ = self.verify(digest=None)
        self.assertEqual(results(result)['ASSET_SHA256'], 'FAIL')

    def test_prerelease_is_unsafe(self):
        files = dict(UPSTREAM)
        manifest = support.build_manifest(files)
        payload = support.make_zip(files, manifest)
        release = support.release_info('v1.6', payload, prerelease=True)
        result = package.verify_package(release, payload)
        self.assertEqual(results(result)['RELEASE_CHANNEL'], 'FAIL')

    def test_missing_manifest_is_unsafe(self):
        result, _, _ = self.verify(include_manifest=False)
        self.assertEqual(results(result)['MANIFEST_SCHEMA'], 'FAIL')
        self.assertEqual(result.status, 'UNSAFE_PACKAGE')

    def test_unparseable_manifest_is_unsafe(self):
        result, _, _ = self.verify(manifest_override=b'{not json')
        self.assertEqual(results(result)['MANIFEST_SCHEMA'], 'FAIL')

    def test_unsupported_schema_version_is_unsafe(self):
        files = dict(UPSTREAM)
        manifest = support.build_manifest(files)
        manifest['schema_version'] = 2
        result, _, _ = self.verify(files, manifest)
        self.assertEqual(results(result)['MANIFEST_SCHEMA'], 'FAIL')

    def test_manifest_entry_missing_hash_is_unsafe(self):
        files = dict(UPSTREAM)
        manifest = support.build_manifest(files)
        del manifest['files'][0]['sha256']
        result, _, _ = self.verify(files, manifest)
        self.assertEqual(results(result)['MANIFEST_SCHEMA'], 'FAIL')

    def test_manifest_listing_itself_is_unsafe(self):
        files = dict(UPSTREAM)
        manifest = support.build_manifest(files)
        manifest['files'].append({'path': 'MANIFEST.json', 'category': 'core',
                                  'sha256': '0' * 64, 'size': 1})
        result, _, _ = self.verify(files, manifest)
        self.assertEqual(results(result)['MANIFEST_SCHEMA'], 'FAIL')

    def test_unsorted_manifest_is_unsafe(self):
        files = dict(UPSTREAM)
        manifest = support.build_manifest(files)
        manifest['files'].reverse()
        result, _, _ = self.verify(files, manifest)
        self.assertEqual(results(result)['MANIFEST_SCHEMA'], 'FAIL')

    def test_category_must_match_path(self):
        files = dict(UPSTREAM)
        manifest = support.build_manifest(files)
        manifest['files'][0]['category'] = 'template'  # AGENTS.md is core
        result, _, _ = self.verify(files, manifest)
        self.assertEqual(results(result)['MANIFEST_SCHEMA'], 'FAIL')

    def test_manifest_sha256_recorded_by_release_must_match(self):
        files = dict(UPSTREAM)
        manifest = support.build_manifest(files)
        payload = support.make_zip(files, manifest)
        release = support.release_info('v1.6', payload, manifest_sha256='0' * 64)
        result = package.verify_package(release, payload)
        self.assertEqual(results(result)['MANIFEST_SHA256'], 'FAIL')

    def test_file_set_mismatch_is_unsafe(self):
        files = dict(UPSTREAM)
        manifest = support.build_manifest({**files, 'kit/extra.md': b'extra\n'})
        result, _, _ = self.verify(files, manifest)
        self.assertEqual(results(result)['FILE_SET'], 'FAIL')

    def test_member_hash_mismatch_is_unsafe(self):
        files = dict(UPSTREAM)
        manifest = support.build_manifest(files)
        files['AGENTS.md'] = b'# tampered\n'
        result, _, _ = self.verify(files, manifest)
        self.assertEqual(results(result)['FILE_SET'], 'FAIL')

    def test_path_traversal_member_is_unsafe(self):
        result, _, _ = self.verify(raw_members=(('agentorch-20260101/../evil.txt', b'x', 0o644),))
        self.assertEqual(results(result)['MEMBER_PATHS'], 'FAIL')

    def test_absolute_and_drive_and_backslash_members_are_unsafe(self):
        for name in ('/etc/passwd', 'C:/evil.txt', 'agentorch-20260101//evil.txt'):
            with self.subTest(name=name):
                result, _, _ = self.verify(raw_members=((name, b'x', 0o644),))
                self.assertEqual(results(result)['MEMBER_PATHS'], 'FAIL')
        # `zipfile` rewrites the native separator to "/" both when writing and
        # when reading an archive, so on Windows a backslash member reaches the
        # verifier already normalized and `MEMBER_PATHS` cannot be the check that
        # fires. The package must still be rejected on both platforms, so assert
        # the exact path check on POSIX and rejection-by-some-path-check on
        # Windows instead of dropping the hostile case.
        result, _, _ = self.verify(
            raw_members=(('agentorch-20260101\\evil.txt', b'x', 0o644),))
        checks = results(result)
        if os.name == 'nt':
            self.assertIn('FAIL', {checks['MEMBER_PATHS'], checks['FILE_SET'],
                                   checks['EXPECTED_LAYOUT']},
                          f'backslash member was accepted: {checks}')
        else:
            self.assertEqual(checks['MEMBER_PATHS'], 'FAIL')

    def test_path_validator_rejects_backslash_on_every_platform(self):
        """The rule itself, asserted directly and identically on every platform.

        The integration case above cannot prove this on Windows: `zipfile`
        rewrites the native separator to "/" when writing *and* when reading an
        archive, so a raw backslash member never reaches the validator there.
        Calling `package_path_errors` directly keeps the invariant
        ("backslash is never a legal member path") asserted on Linux, macOS and
        Windows instead of only on the platforms where the fixture survives.
        """
        errors = package.package_path_errors('agentorch-20260101\\evil.txt')
        self.assertTrue(any('no backslash' in error for error in errors),
                        f'backslash path was not rejected: {errors}')
        # Control: the same path with the only legal separator stays valid, so a
        # validator that rejected everything could not satisfy the assertion above.
        self.assertEqual(package.package_path_errors('agentorch-20260101/evil.txt'), [])

    def test_symlink_member_is_unsafe(self):
        result, _, _ = self.verify(
            raw_members=(('agentorch-20260101/kit/link', b'AGENTS.md', 0o120777),))
        self.assertEqual(results(result)['SYMLINKS'], 'FAIL')

    def test_executable_member_is_unsafe(self):
        files = dict(UPSTREAM)
        manifest = support.build_manifest(files)
        payload = support.make_zip(files, manifest)
        # rewrite AGENTS.md with an executable mode, keeping the same bytes
        with zipfile.ZipFile(io.BytesIO(payload)) as archive:
            entries = [(info.filename, archive.read(info)) for info in archive.infolist()]
        buffer = io.BytesIO()
        with zipfile.ZipFile(buffer, 'w') as archive:
            for name, data in entries:
                mode = 0o755 if name.endswith('AGENTS.md') else 0o644
                support.write_member(archive, name, data, mode)
        release = support.release_info('v1.6', buffer.getvalue())
        result = package.verify_package(release, buffer.getvalue())
        self.assertEqual(results(result)['EXPECTED_LAYOUT'], 'FAIL')

    def test_nested_archive_is_unsafe(self):
        files = dict(UPSTREAM)
        files['kit/bundle.zip'] = b'PK\x03\x04fake'
        manifest = support.build_manifest(files)
        result, _, _ = self.verify(files, manifest)
        self.assertEqual(results(result)['EXPECTED_LAYOUT'], 'FAIL')

    def test_flat_archive_without_single_top_directory_is_unsafe(self):
        # The manifest location itself pins the single top-level directory, so a
        # flat package fails closed at MANIFEST_SCHEMA (no later check is reached).
        result, _, _ = self.verify(top=None)
        self.assertFalse(result.ok)
        self.assertEqual(results(result)['MANIFEST_SCHEMA'], 'FAIL')

    def test_case_colliding_members_are_unsafe(self):
        files = {'kit/A.md': b'a\n', 'kit/a.md': b'b\n'}
        manifest = support.build_manifest(files)
        result, _, _ = self.verify(files, manifest)
        self.assertEqual(results(result)['EXPECTED_LAYOUT'], 'FAIL')

    def test_member_count_limit_is_enforced(self):
        files = dict(UPSTREAM)
        manifest = support.build_manifest(files)
        limits = package.PackageLimits(max_members=2)
        result, _, _ = self.verify(files, manifest, limits=limits)
        self.assertEqual(results(result)['MEMBER_PATHS'], 'FAIL')

    def test_uncompressed_size_limit_is_enforced(self):
        files = dict(UPSTREAM)
        manifest = support.build_manifest(files)
        limits = package.PackageLimits(max_total_uncompressed=4)
        result, _, _ = self.verify(files, manifest, limits=limits)
        self.assertEqual(results(result)['MEMBER_PATHS'], 'FAIL')

    def test_compression_ratio_limit_is_enforced(self):
        files = {'kit/big.md': b'0' * 200_000}
        manifest = support.build_manifest(files)
        limits = package.PackageLimits(max_compression_ratio=2)
        result, _, _ = self.verify(files, manifest, limits=limits)
        self.assertEqual(results(result)['MEMBER_PATHS'], 'FAIL')

    def test_version_must_match_manifest_format(self):
        result, _, _ = self.verify(tag='v1.7', manifest_format='1.6')
        self.assertEqual(results(result)['VERSION_MATCH'], 'FAIL')

    def test_suite_must_match_baseline(self):
        result, _, _ = self.verify(expected_suite='Another Suite')
        self.assertEqual(results(result)['VERSION_MATCH'], 'FAIL')

    def test_two_and_three_part_tags_are_accepted(self):
        for tag, version in (('v1.6', '1.6.0'), ('v1.6.0', '1.6.0'), ('v1.6.3', '1.6.3')):
            with self.subTest(tag=tag):
                result, _, _ = self.verify(tag=tag)
                self.assertTrue(result.ok)
                self.assertEqual(result.version, version)

    def test_unexecuted_checks_are_not_run_after_a_failure(self):
        result, _, _ = self.verify(digest='sha256:' + '0' * 64)
        results_map = results(result)
        self.assertEqual(results_map['SOURCE_ALLOWLIST'], 'PASS')
        self.assertEqual(results_map['ASSET_SHA256'], 'FAIL')
        # nothing after the failing check may be reported as PASS
        self.assertNotIn('PASS', [v for k, v in results_map.items()
                                  if package.CHECK_ORDER.index(k) > package.CHECK_ORDER.index('ASSET_SHA256')])
        self.assertEqual(results_map['FILE_SET'], 'NOT_RUN')

    def test_default_limits_are_finite_and_documented(self):
        self.assertLess(package.DEFAULT_LIMITS.max_members, 100_000)
        self.assertLess(package.DEFAULT_LIMITS.max_total_uncompressed, 1 << 30)
        self.assertLess(package.DEFAULT_LIMITS.max_member_uncompressed, 1 << 30)
        self.assertGreater(package.DEFAULT_LIMITS.max_compression_ratio, 1)


if __name__ == '__main__':
    unittest.main()
