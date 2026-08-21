# Security Policy

## Reporting a Vulnerability

If you discover a security vulnerability, please report it responsibly:

1. **Do NOT** open a public GitHub issue
2. Email: tushar@wellarrive.com (or use GitHub's private vulnerability reporting)
3. Include: description, reproduction steps, impact assessment
4. We will acknowledge within 48 hours and provide a fix timeline

## Supported Versions

| Version | Supported |
|---------|----------|
| latest  | ✅ |
| < latest | ❌ |

## Validator Secrets

Never include a validation seed, node-private key, RFC-1751 validation words,
validator token, or `validator-keys.json` in a report. Redact credentials and
private endpoints from configuration and logs. If a validator secret may have
been exposed, stop using that identity and contact the maintainers privately
before publishing details.
