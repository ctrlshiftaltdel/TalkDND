"""Optional Architecture Contract checker (Issue #48).

Pure standard library. Entry point: `scripts/architecture/check.py`.
Contract semantics are frozen in `docs/architecture/architecture-contract.md`
and `docs/contracts/schemas/architecture-contract.schema.json` (Issue #47);
this package only implements them and never invents module boundaries.
"""

__all__ = ['check', 'contract', 'report', 'rules']
