"""Experience-source registry and lesson evidence pipeline (#49).

Four modules, one direction of dependency:

``sources``   reads / writes / validates the single authoritative source
              registry (``docs/self-improvement/sources.json``). Source *entries*
              are configuration data; nothing here branches on a source_id.
``collect``   read-only evidence collection through the ``gh`` CLI. Failures
              become ``BLOCKED`` / ``UNKNOWN``; nothing is ever written to a
              source repository.
``lessons``   lesson records, content classes, evidence levels, the frozen state
              machine, the ``sha256-normalized-v1`` fingerprint, dedup and
              conflict handling, and report rendering.
``pipeline``  wires the three together for one scan; ``cli`` is the command line.
``planning``  routes a user trigger and gates improvement proposals (#50).
``orchestration``  runs one trigger → restore → scan → plan → register → report
              pass, with an injected carrier and no approval / merge step (#50).

Every string that comes from a source is *data*. It is carried in explicit data
fields (``title``, ``labels`` ...), never rendered as an instruction, and never
used to change the registry, the allow-list or the task/acceptance rules.

Frozen semantics are defined in ``docs/self-improvement/sources-and-trust.md``,
``docs/self-improvement/lesson-lifecycle.md`` and the two schemas under
``docs/contracts/schemas/``. This package implements them; it does not extend
them. Standard library only.
"""

__all__ = ['sources', 'collect', 'lessons', 'pipeline', 'planning', 'orchestration']
