# Spec: legacy-migration

Explicit outcomes for one-time legacy migration and stable-directory initialization.

## ADDED Requirements

### Requirement: Migration outcomes are explicit

Startup SHALL distinguish at least: no legacy source exists, migration succeeded, a legacy source exists but migration was blocked or failed, and stable data already present making migration unnecessary. A blocked migration SHALL leave the legacy source untouched and be reported with its exact source path and failure reason.

#### Scenario: Failed destination write is reported, not hidden

- **WHEN** legacy data exists but cannot be copied to the stable location
- **THEN** startup reports the failed source path instead of presenting a clean first launch, and the legacy file remains unchanged

#### Scenario: Successful migration verifies before rename

- **WHEN** legacy data is copied to the stable location
- **THEN** the stable copy is verified readable before the legacy source is renamed aside as migrated

### Requirement: Stable-directory failures are surfaced

Failure to create or access the stable data directory SHALL be reported to the user rather than silently falling back to an unexpected location or deferring to later write failures.

#### Scenario: Data directory cannot be created

- **WHEN** the stable data directory cannot be created at startup
- **THEN** the app reports the target path and the failure reason
