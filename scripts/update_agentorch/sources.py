"""Official release source allowlist and the GitHub Releases client.

Frozen rule (docs/self-improvement/update-safety.md §2): the only accepted
source is a GitHub Release of ``ctrlshiftaltdel/AgentOrch``. The repository is a
module constant, not configuration, so a project cannot point the updater at a
fork, a branch build, an Issue attachment or an arbitrary URL.

The network client is intentionally thin. Every raw release object -- whether it
came from the GitHub API or from a synthetic fixture in the tests -- goes through
the same :func:`select_asset` validation. Network failures never fall back to a
guessed package.
"""
from __future__ import annotations

import json
import os
import re
import urllib.error
import urllib.request
from dataclasses import dataclass
from urllib.parse import urlparse

from . import versions

OFFICIAL_REPOSITORY = 'ctrlshiftaltdel/AgentOrch'
OFFICIAL_API = 'https://api.github.com'
OFFICIAL_HOSTS = frozenset({'github.com', 'api.github.com'})
DOWNLOAD_HOST_SUFFIXES = ('.githubusercontent.com',)

ASSET_NAME_RE = re.compile(r'^[A-Za-z0-9._-]+\.zip$')
DIGEST_RE = re.compile(r'^sha256:([a-f0-9]{64})$')

USER_AGENT = 'agentorch-update-engine'
MAX_METADATA_BYTES = 1024 * 1024
MAX_ASSET_BYTES = 64 * 1024 * 1024


class SourceError(Exception):
    """A release record that must not be trusted.

    ``check`` is one of the frozen verification check names so the caller can
    map the failure onto ``target.verification.checks`` without re-classifying.
    """

    def __init__(self, check: str, detail: str):
        super().__init__(f'{check}: {detail}')
        self.check = check
        self.detail = detail


@dataclass(frozen=True)
class ReleaseAsset:
    name: str
    size: int
    digest: str | None
    download_url: str


@dataclass(frozen=True)
class ReleaseInfo:
    repository: str
    tag_name: str
    assets: tuple[ReleaseAsset, ...] = ()
    prerelease: bool = False
    draft: bool = False
    html_url: str | None = None
    # Optional independent digest of the package MANIFEST.json. GitHub does not
    # publish one today, so it stays None unless a provider can supply it.
    manifest_sha256: str | None = None


def digest_hex(digest: str | None) -> str | None:
    if not isinstance(digest, str):
        return None
    match = DIGEST_RE.match(digest)
    return match.group(1) if match else None


def _validate_release_url(url: str | None, repository: str) -> None:
    if not url:
        return
    parsed = urlparse(url)
    if parsed.scheme != 'https' or parsed.hostname not in OFFICIAL_HOSTS:
        raise SourceError('SOURCE_ALLOWLIST', f'release URL host is not the official host: {parsed.netloc or url!r}')
    expected = f'/{repository}/releases'
    if not parsed.path.startswith(expected):
        raise SourceError('SOURCE_ALLOWLIST',
                          f'release URL does not point at {repository} Releases: {parsed.path!r}')


def _validate_download_url(url: str, repository: str) -> None:
    parsed = urlparse(url)
    host = parsed.hostname or ''
    allowed_host = host in OFFICIAL_HOSTS or host.endswith(DOWNLOAD_HOST_SUFFIXES)
    if parsed.scheme != 'https' or not allowed_host:
        raise SourceError('SOURCE_ALLOWLIST', f'asset download host is not an official GitHub host: {host!r}')
    if host in OFFICIAL_HOSTS:
        # github.com/api.github.com URLs must still name the official repository.
        if not parsed.path.startswith(f'/repos/{repository}/') and \
                not parsed.path.startswith(f'/{repository}/releases/'):
            raise SourceError('SOURCE_ALLOWLIST',
                              f'asset URL does not point at {repository}: {parsed.path!r}')


def select_asset(release: ReleaseInfo) -> ReleaseAsset:
    """Validate a release record and return its single official zip asset."""
    if release.repository != OFFICIAL_REPOSITORY:
        raise SourceError('SOURCE_ALLOWLIST',
                          f'repository must be {OFFICIAL_REPOSITORY}, got {release.repository!r}')
    _validate_release_url(release.html_url, OFFICIAL_REPOSITORY)
    if not versions.parse_tag(release.tag_name):
        raise SourceError('SOURCE_ALLOWLIST',
                          f'release tag is not v<major>.<minor>[.<patch>]: {release.tag_name!r}')
    if release.prerelease or release.draft:
        channel = 'draft' if release.draft else 'prerelease'
        raise SourceError('RELEASE_CHANNEL', f'release is a {channel}; only stable releases are accepted')

    zips = [asset for asset in release.assets if isinstance(asset.name, str) and asset.name.endswith('.zip')]
    if len(zips) != 1:
        raise SourceError('SOURCE_ALLOWLIST',
                          f'release must expose exactly one .zip asset, found {len(zips)}')
    asset = zips[0]
    if not ASSET_NAME_RE.match(asset.name) or '/' in asset.name or '\\' in asset.name:
        raise SourceError('SOURCE_ALLOWLIST', f'asset name is not a plain .zip name: {asset.name!r}')
    if not isinstance(asset.size, int) or isinstance(asset.size, bool) or asset.size < 1:
        raise SourceError('SOURCE_ALLOWLIST', f'asset size is missing or invalid: {asset.size!r}')
    if not asset.download_url:
        raise SourceError('SOURCE_ALLOWLIST', 'asset has no download URL')
    _validate_download_url(asset.download_url, OFFICIAL_REPOSITORY)
    return asset


def read_limited(stream, limit: int, *, check: str = 'ASSET_SHA256') -> bytes:
    """Read at most ``limit`` bytes; a larger body is an unsafe package."""
    chunks: list[bytes] = []
    total = 0
    while True:
        chunk = stream.read(min(64 * 1024, limit - total + 1))
        if not chunk:
            break
        total += len(chunk)
        if total > limit:
            raise SourceError(check, f'download exceeds the {limit} byte limit')
        chunks.append(chunk)
    return b''.join(chunks)


def parse_release_payload(payload: dict, repository: str = OFFICIAL_REPOSITORY) -> ReleaseInfo:
    """Turn a GitHub ``/releases/latest`` JSON object into a :class:`ReleaseInfo`."""
    assets = []
    for entry in payload.get('assets') or []:
        if not isinstance(entry, dict):
            continue
        try:
            size = int(entry.get('size') or 0)
        except (TypeError, ValueError):
            size = 0
        assets.append(ReleaseAsset(
            name=str(entry.get('name') or ''),
            size=size,
            digest=entry.get('digest'),
            download_url=str(entry.get('browser_download_url') or entry.get('url') or ''),
        ))
    return ReleaseInfo(
        repository=repository,
        tag_name=str(payload.get('tag_name') or ''),
        assets=tuple(assets),
        prerelease=bool(payload.get('prerelease')),
        draft=bool(payload.get('draft')),
        html_url=payload.get('html_url'),
    )


class GitHubReleaseSource:
    """Read the latest stable release of the official repository.

    There is deliberately no repository parameter: the source is pinned by the
    frozen contract. ``GITHUB_TOKEN`` is read from the environment when present
    (public repositories work without it) and is never written to a report,
    log or command entry.
    """

    def __init__(self, token: str | None = None, timeout: float = 30.0):
        self._token = token if token is not None else os.environ.get('GITHUB_TOKEN')
        self._timeout = timeout

    def _request(self, url: str, *, authenticate: bool, limit: int) -> bytes:
        headers = {'Accept': 'application/vnd.github+json', 'User-Agent': USER_AGENT}
        if authenticate and self._token:
            headers['Authorization'] = f'Bearer {self._token}'
        request = urllib.request.Request(url, headers=headers)
        with urllib.request.urlopen(request, timeout=self._timeout) as response:
            return read_limited(response, limit)

    def fetch_release(self) -> ReleaseInfo:
        url = f'{OFFICIAL_API}/repos/{OFFICIAL_REPOSITORY}/releases/latest'
        raw = self._request(url, authenticate=True, limit=MAX_METADATA_BYTES)
        try:
            payload = json.loads(raw.decode('utf-8'))
        except (UnicodeDecodeError, ValueError) as exc:
            raise SourceError('SOURCE_ALLOWLIST', f'release metadata is not valid JSON: {exc}')
        if not isinstance(payload, dict):
            raise SourceError('SOURCE_ALLOWLIST', 'release metadata is not a JSON object')
        return parse_release_payload(payload)

    def download_asset(self, asset: ReleaseAsset) -> bytes:
        _validate_download_url(asset.download_url, OFFICIAL_REPOSITORY)
        authenticate = (urlparse(asset.download_url).hostname or '') in {'api.github.com', 'github.com'}
        return self._request(asset.download_url, authenticate=authenticate, limit=MAX_ASSET_BYTES)
