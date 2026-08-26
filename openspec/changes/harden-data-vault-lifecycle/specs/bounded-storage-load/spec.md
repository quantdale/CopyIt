# Spec: bounded-storage-load

Size-bounded reads of JSON data files before allocation or parsing.

## ADDED Requirements

### Requirement: Data files are read under a hard byte limit

Loading a JSON data file SHALL read at most limit-plus-one bytes from disk and SHALL reject any file that exceeds the configured maximum (256 MiB) without reading the remainder into memory. The bound SHALL NOT depend on file metadata alone, and no buffer SHALL be allocated from untrusted file sizes.

#### Scenario: Oversized file is rejected cheaply

- **WHEN** a data file is larger than the configured limit
- **THEN** the loader reports the size violation as an unreadable file after reading at most limit-plus-one bytes

#### Scenario: File at exactly the limit loads

- **WHEN** a data file's byte length equals the configured limit
- **THEN** it is parsed normally if valid JSON

### Requirement: Load semantics are preserved under bounded reads

Zero-byte and whitespace-only files SHALL report missing, invalid UTF-8 or JSON SHALL report corrupt, and valid documents within the limit SHALL load unchanged.

#### Scenario: Small-file behaviors are unchanged

- **WHEN** loading empty, whitespace-only, malformed, or valid files well under the limit
- **THEN** outcomes match the pre-existing missing/corrupt/loaded classification
