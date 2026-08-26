# Spec: settings-persistence

First-class surfacing of config-save failures so ordinary settings changes never silently claim durability they don't have.

## ADDED Requirements

### Requirement: Config-write failures are surfaced, not discarded

Theme selection and category creation SHALL propagate `save_config` failures into a durable warning naming what was not persisted, instead of discarding the result.

#### Scenario: Theme change fails to persist

- **WHEN** the user selects a different theme and the config write fails
- **THEN** the new theme stays active in memory but a warning states it will revert on restart

#### Scenario: Category registration fails to persist

- **WHEN** a new category is registered and the config write fails
- **THEN** a warning states the setting was not persisted and the category list remains usable this session

### Requirement: Warnings are deduplicated and precisely retired

Repeated failures within a session SHALL NOT grow duplicate copies of the same warning, and a later successful config save SHALL retire only the config-save warnings it actually repairs — never corruption-recovery notices or snippet-save errors.

#### Scenario: Repeated frames do not duplicate the warning

- **WHEN** several consecutive saves fail with the same error
- **THEN** the banner shows the message once

#### Scenario: Success retires only its own class

- **WHEN** a config save succeeds while a snippets-save error and a recovery notice are also displayed
- **THEN** only the config-save warning is cleared
