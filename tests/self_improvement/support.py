"""Synthetic fixtures shared by the #49 tests.

Everything here is invented: no source repository text, no real PR numbers that
matter, no credentials that work. The shapes mirror the GitHub REST payloads the
collector reads, so the tests exercise field extraction, failure classification
and redaction without a network or a token.
"""
import importlib.util
import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
_SCRIPTS = ROOT / 'scripts'

# ``unittest discover -s tests`` imports this package as top-level
# ``self_improvement``, which would shadow ``scripts/self_improvement``. Load the
# implementation package under an explicit alias instead of relying on sys.path
# order; relative imports inside it still resolve to the same directory.
if 'agentorch_self_improvement' not in sys.modules:
    _spec = importlib.util.spec_from_file_location(
        'agentorch_self_improvement',
        _SCRIPTS / 'self_improvement' / '__init__.py',
        submodule_search_locations=[str(_SCRIPTS / 'self_improvement')])
    _module = importlib.util.module_from_spec(_spec)
    sys.modules['agentorch_self_improvement'] = _module
    _spec.loader.exec_module(_module)

from agentorch_self_improvement import collect, lessons, pipeline, sources  # noqa: E402

# Credential-shaped fixture values, assembled so this file does not contain the
# consecutive prefixes the release scan forbids (see scripts/build_release.py).
TOKEN_PREFIX = 'gh' + 'p_'
FAKE_TOKEN = TOKEN_PREFIX + 'A' * 24

SHA_A = 'a' * 40
SHA_B = 'b' * 40
SHA_C = 'c' * 40

T0 = '2026-08-01T00:00:00Z'
T1 = '2026-09-01T00:00:00Z'
T2 = '2026-10-20T00:00:00Z'


def trust(allowed_use=('evidence_only', 'synthetic_fixture_source')):
    return {
        'content_trust': 'untrusted_data',
        'allowed_use': list(allowed_use),
        'redaction_required': True,
        'may_override_rules': False,
    }


def source(source_id='talkdnd', repository='ctrlshiftaltdel/TalkDND', **overrides):
    payload = {
        'source_id': source_id,
        'repository': repository,
        'enabled': True,
        'source_type': 'downstream_project',
        'access_mode': 'public',
        'evidence_scope': ['pull_request', 'issue', 'review', 'commit', 'ci_run'],
        'lookback_days': 90,
        'scan_cursor': None,
        'trust': trust(),
        'inclusion_filters': [],
        'exclusion_filters': [],
    }
    payload.update(overrides)
    return payload


def registry(*entries, updated_at=T2):
    return {'schema_version': '1.0', 'registry_updated_at': updated_at,
            'sources': list(entries) or [source()]}


def observation(**overrides):
    payload = {
        'observed_fact': '来源项目的一个合并 PR 缺少对应回归测试，缺陷在后续版本再次出现。',
        'repro_conditions': 'not reproducible here: 只读取来源侧合并记录与 CI 结果，未在隔离环境复现。',
        'root_cause': '合并门只检查 CI 通过，没有要求新增回归测试覆盖该路径。',
        'impact': '同类缺陷在后续版本重复出现，且没有测试能在合并前拦截。',
        'why_missed': '来源项目的审查清单没有把回归测试列为合并前必需证据。',
    }
    payload.update(overrides)
    return payload


def analysis(**overrides):
    payload = {
        'title': '合并门缺少回归测试证据时缺陷会重复出现',
        'content_class': 'CONFIRMED_FACT',
        'evidence_level': 'E1_SOURCE_ARTIFACT',
        'observation': observation(),
        'applicability': {'scope': 'source_only',
                          'applies_to': ['scripts/self_improvement/lessons.py'],
                          'excluded_contexts': ['没有 CI 的项目']},
        'suggested_action': {'target': 'TEST',
                             'summary': '在合并门证据矩阵里要求一条回归测试证据。',
                             'cost': 'LOW', 'risk': 'MEDIUM',
                             'validation_command': None},
    }
    payload.update(overrides)
    return payload


def lesson(**overrides):
    params = {
        'lesson_id': 'LSN-TALKDND-001',
        'title': '合并门缺少回归测试证据时缺陷会重复出现',
        'content_class': 'CONFIRMED_FACT',
        'evidence_level': 'E1_SOURCE_ARTIFACT',
        'status': 'OBSERVED',
        'source': {
            'source_id': 'talkdnd',
            'evidence_kind': 'pull_request',
            'evidence_urls': ['https://github.com/ctrlshiftaltdel/TalkDND/pull/12'],
            'observed_at': T1,
            'captured_at': T2,
            'source_commit': SHA_A,
        },
        'observation': observation(),
        'applicability': {'scope': 'source_only',
                          'applies_to': ['scripts/self_improvement/lessons.py'],
                          'excluded_contexts': ['没有 CI 的项目']},
        'suggested_action': {'target': 'TEST',
                             'summary': '在合并门证据矩阵里要求一条回归测试证据。',
                             'cost': 'LOW', 'risk': 'MEDIUM',
                             'validation_command': None},
        'created_at': T2,
        'updated_at': T2,
    }
    params.update(overrides)
    return lessons.build_lesson(**params)


def raw_lesson(**overrides):
    """A valid record with a field overridden *without* revalidation.

    Used only by tests that must inspect validation errors for a shape the
    builder would refuse outright.
    """
    import copy
    record = copy.deepcopy(lesson())
    record.update(overrides)
    return record


def verified_lesson(**overrides):
    payload = {'status': 'VERIFIED', 'evidence_level': 'E2_REPRODUCED',
               'content_class': 'CONFIRMED_FACT'}
    payload.update(overrides)
    return lesson(**payload)


def decision(outcome='ADOPT', decided_by='project-owner'):
    return lessons.make_decision(decided_by, outcome,
                                 '已复核来源证据与适用条件，决定按该结论处理。')


def pr_payload(number=12, title='Add regression test', merged=True, sha=SHA_A,
               labels=(), state=None):
    return {
        'number': number,
        'html_url': f'https://github.com/ctrlshiftaltdel/TalkDND/pull/{number}',
        'title': title,
        'state': state or ('closed' if merged else 'open'),
        'merged_at': T1 if merged else None,
        'created_at': T1,
        'updated_at': T1,
        'head': {'sha': sha},
        'labels': [{'name': name} for name in labels],
    }


def issue_payload(number=7, title='Flaky merge gate', state='closed', labels=()):
    return {
        'number': number,
        'html_url': f'https://github.com/ctrlshiftaltdel/TalkDND/issues/{number}',
        'title': title,
        'state': state,
        'created_at': T1,
        'updated_at': T1,
        'labels': [{'name': name} for name in labels],
    }


def review_payload(pr=12, sha=SHA_A):
    return {
        'html_url': f'https://github.com/ctrlshiftaltdel/TalkDND/pull/{pr}#discussion_r1',
        'body': 'this body must never be stored',
        'created_at': T1,
        'updated_at': T1,
        'commit_id': sha,
        'pull_request_url': f'https://api.github.com/repos/ctrlshiftaltdel/TalkDND/pulls/{pr}',
    }


def commit_payload(sha=SHA_A, message='Fix gate ordering', date=T1):
    return {
        'sha': sha,
        'html_url': f'https://github.com/ctrlshiftaltdel/TalkDND/commit/{sha}',
        'commit': {'message': message, 'author': {'date': date}},
    }


def ci_run_payload(sha=SHA_A, conclusion='success'):
    return {
        'name': 'ci',
        'html_url': f'https://github.com/ctrlshiftaltdel/TalkDND/actions/runs/1',
        'status': 'completed',
        'conclusion': conclusion,
        'created_at': T1,
        'updated_at': T1,
        'head_sha': sha,
    }


def github_payloads(**overrides):
    """Endpoint marker -> synthetic REST payload for one repository."""
    payloads = {
        '/pulls/comments': [review_payload()],
        '/pulls?': [pr_payload()],
        '/issues?': [issue_payload()],
        '/commits?': [commit_payload()],
        '/actions/runs': [ci_run_payload()],
    }
    payloads.update(overrides)
    return payloads


class FakeRunner(collect.CommandRunner):
    """Deterministic ``gh`` stand-in. Records every argv it was asked to run."""

    def __init__(self, payloads=None, failure=None):
        self.payloads = dict(payloads or github_payloads())
        self.failure = failure or {}
        self.commands = []

    def run(self, args):
        self.commands.append(list(args))
        command = list(args)
        path = command[-1]
        for marker, failure in self.failure.items():
            if marker in path:
                if isinstance(failure, Exception):
                    raise failure
                if callable(failure):
                    return failure(path)
                return failure
        for marker, payload in self.payloads.items():
            if marker in path:
                if callable(payload):
                    return 0, json.dumps(payload(path)), ''
                if isinstance(payload, tuple) and len(payload) == 3:
                    return payload  # raw (returncode, stdout, stderr)
                return 0, json.dumps(payload), ''
        return 0, '[]', ''


def registry_file(tmp_dir, *entries, **kwargs):
    path = Path(tmp_dir) / 'sources.json'
    path.write_text(json.dumps(registry(*entries, **kwargs), ensure_ascii=False),
                    encoding='utf-8')
    return path
