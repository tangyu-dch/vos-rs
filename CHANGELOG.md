# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- **CI/CD Workflows**: Added `.github/workflows/ci.yml` and `.github/workflows/release.yml` for automated Cargo clippy, formatting, testing, React frontend build checks, and Semantic Docker releases.
- **Security Hardening**: Added `.env.example` and `.env.production` templates; added `scripts/gen-tls-certs.sh` for one-click self-signed TLS certificates generation; sanitized S3 credentials in `config.yaml`.
- **Environment Management**: Added `docker-compose.dev.yml` and `docker-compose.prod.yml` with cgroups resource limits and health probes.
- **Readiness Probes**: Enhanced `/ready` HTTP endpoint on `api-server` to return structured JSON component status (PostgreSQL connection pool status).
- **Engineering Quality**: Added `.pre-commit-config.yaml` and `scripts/install-hooks.sh` to enforce static analysis before commits.
- **Copilot Multimodal Improvements**: Added persistent `images TEXT[]` array storage to PostgreSQL `copilot_messages` schema, Lightbox image preview modal, and interactive CSV Table Data Viewer Modal.

### Fixed
- **API Audit Log Buffer**: Increased body buffer to 15MB to prevent HTTP 400 EOF errors during large Base64 multimodal image uploads.
- **Persistent Chat History**: Fixed loss of user-uploaded images in Copilot session bubbles after page refresh.

## [v0.1.0] - 2026-07-24

- Initial product-grade Rust Telecom VoIP Softswitch release supporting 5000+ CPS / 1500+ Concurrent Calls.
- B2BUA SIP engine, RTP Media relay, NATS JetStream CDR Worker, RESTful API Server, and React Admin Operations Console.
