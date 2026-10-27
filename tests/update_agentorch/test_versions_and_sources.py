"""Version normalization and official-source allowlist tests.

The source tests never touch the network: they exercise the pure selection and
parsing functions with synthetic GitHub release payloads.
"""
from __future__ import annotations

import inspect
import unittest

from . import support

sources = support.sources
versions = support.versions


class VersionTests(unittest.TestCase):
    def test_two_part_tag_is_accepted_and_patch_defaults_to_zero(self):
        self.assertEqual(versions.parse_tag('v1.6'), (1, 6, 0))
        self.assertEqual(versions.parse_tag('v1.6.2'), (1, 6, 2))
        self.assertEqual(versions.normalize_version('v1.6'), '1.6.0')
        self.assertEqual(versions.normalize_version('1.6.2'), '1.6.2')

    def test_rejects_other_tag_shapes(self):
        for tag in ('1.6', 'v1', 'v1.6.0.1', 'v1.6-rc1', 'release-1.6', '', 'v01.6'):
            with self.subTest(tag=tag):
                self.assertIsNone(versions.parse_tag(tag))

    def test_version_order(self):
        self.assertLess(versions.version_key('1.5'), versions.version_key('1.6'))
        self.assertEqual(versions.version_key('v1.6'), versions.version_key('1.6.0'))
        self.assertLess(versions.version_key('1.6'), versions.version_key('1.6.1'))

    def test_tag_matches_manifest_format(self):
        self.assertTrue(versions.tag_matches_format('v1.6', '1.6'))
        self.assertTrue(versions.tag_matches_format('v1.6.3', '1.6'))
        self.assertFalse(versions.tag_matches_format('v1.7', '1.6'))
        self.assertFalse(versions.tag_matches_format('nope', '1.6'))


class SourceTests(unittest.TestCase):
    def test_allowlist_constant_is_the_official_repository(self):
        self.assertEqual(sources.OFFICIAL_REPOSITORY, 'ctrlshiftaltdel/AgentOrch')
        # The repository must not be overridable through the production client.
        self.assertNotIn('repository', inspect.signature(sources.GitHubReleaseSource).parameters)

    def test_selects_the_single_zip_asset(self):
        payload = b'zip bytes'
        release = support.release_info('v1.6', payload)
        asset = sources.select_asset(release)
        self.assertEqual(asset.name, 'agentorch-20260101.zip')
        self.assertTrue(asset.digest.startswith('sha256:'))

    def test_rejects_non_official_repository(self):
        release = support.release_info('v1.6', b'x', repository='someone/AgentOrch')
        with self.assertRaises(sources.SourceError) as caught:
            sources.select_asset(release)
        self.assertEqual(caught.exception.check, 'SOURCE_ALLOWLIST')

    def test_rejects_non_github_download_url(self):
        release = support.release_info('v1.6', b'x')
        bad = sources.ReleaseAsset(name=release.assets[0].name, size=1,
                                   digest=release.assets[0].digest,
                                   download_url='https://evil.example/agentorch.zip')
        release = sources.ReleaseInfo(repository=release.repository, tag_name=release.tag_name,
                                      assets=(bad,), html_url=release.html_url)
        with self.assertRaises(sources.SourceError) as caught:
            sources.select_asset(release)
        self.assertEqual(caught.exception.check, 'SOURCE_ALLOWLIST')

    def test_rejects_prerelease_and_draft(self):
        for kwargs in ({'prerelease': True}, {'draft': True}):
            with self.subTest(**kwargs):
                release = support.release_info('v1.6', b'x', **kwargs)
                with self.assertRaises(sources.SourceError) as caught:
                    sources.select_asset(release)
                self.assertEqual(caught.exception.check, 'RELEASE_CHANNEL')

    def test_rejects_zero_or_multiple_zip_assets(self):
        release = support.release_info('v1.6', b'x')
        no_zip = sources.ReleaseInfo(repository=release.repository, tag_name='v1.6', assets=())
        with self.assertRaises(sources.SourceError) as caught:
            sources.select_asset(no_zip)
        self.assertEqual(caught.exception.check, 'SOURCE_ALLOWLIST')

        extra = support.release_info(
            'v1.6', b'x',
            extra_assets=({'name': 'agentorch-20260102.zip', 'size': 1,
                           'digest': 'sha256:' + '0' * 64,
                           'download_url': 'https://github.com/x/y/releases/download/v1.6/b.zip'},))
        with self.assertRaises(sources.SourceError) as caught:
            sources.select_asset(extra)
        self.assertEqual(caught.exception.check, 'SOURCE_ALLOWLIST')

    def test_rejects_asset_name_with_path_or_missing_extension(self):
        release = support.release_info('v1.6', b'x')
        bad_asset = sources.ReleaseAsset(name='nested/agentorch.zip', size=1,
                                         digest=release.assets[0].digest,
                                         download_url=release.assets[0].download_url)
        custom = sources.ReleaseInfo(repository=release.repository, tag_name='v1.6',
                                     assets=(bad_asset,))
        with self.assertRaises(sources.SourceError):
            sources.select_asset(custom)

    def test_rejects_invalid_tag_shape(self):
        release = support.release_info('1.6', b'x')
        with self.assertRaises(sources.SourceError) as caught:
            sources.select_asset(release)
        self.assertEqual(caught.exception.check, 'SOURCE_ALLOWLIST')

    def test_parse_release_payload(self):
        payload = {
            'tag_name': 'v1.6.1',
            'prerelease': False,
            'draft': False,
            'html_url': 'https://github.com/ctrlshiftaltdel/AgentOrch/releases/tag/v1.6.1',
            'assets': [{
                'name': 'agentorch-20260101.zip',
                'size': 1234,
                'digest': 'sha256:' + 'a' * 64,
                'browser_download_url': 'https://github.com/ctrlshiftaltdel/AgentOrch/releases/download/v1.6.1/agentorch-20260101.zip',
            }],
        }
        release = sources.parse_release_payload(payload)
        self.assertEqual(release.repository, sources.OFFICIAL_REPOSITORY)
        self.assertEqual(release.tag_name, 'v1.6.1')
        self.assertEqual(release.assets[0].digest, 'sha256:' + 'a' * 64)
        self.assertEqual(release.assets[0].size, 1234)

    def test_download_size_limit_is_enforced(self):
        import io

        self.assertEqual(sources.read_limited(io.BytesIO(b'x' * 10), 10), b'x' * 10)
        with self.assertRaises(sources.SourceError) as caught:
            sources.read_limited(io.BytesIO(b'x' * 11), 10)
        self.assertEqual(caught.exception.check, 'ASSET_SHA256')
        self.assertLess(sources.MAX_ASSET_BYTES, 1 << 30)

    def test_parse_missing_digest_is_kept_as_none(self):
        payload = {
            'tag_name': 'v1.6',
            'html_url': 'https://github.com/ctrlshiftaltdel/AgentOrch/releases/tag/v1.6',
            'assets': [{'name': 'agentorch-20260101.zip', 'size': 1,
                        'browser_download_url': 'https://github.com/ctrlshiftaltdel/AgentOrch/releases/download/v1.6/agentorch-20260101.zip'}],
        }
        release = sources.parse_release_payload(payload)
        self.assertIsNone(release.assets[0].digest)


if __name__ == '__main__':
    unittest.main()
