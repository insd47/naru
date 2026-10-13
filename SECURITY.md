# Security

Naru signs in to Macs with their users' passwords, keeps those passwords and the Macs' keys on the device, and sends
keyboard, pointer, and clipboard input to the Mac. Problems in any of these are security issues.

## Reporting

Report a vulnerability privately through
[GitHub's private vulnerability reporting](https://github.com/insd47/naru/security/advisories/new), not in a public
issue. Include the platform, the app version, the macOS version, and the steps that show the problem.

> Do not attach screen captures, passwords, or logs from a real Mac. Describe what they showed instead.

## Scope

- The core: authentication, the encrypted control channel, SRTP, and parsing anything the Mac sends
- Each app's storage of passwords and server keys
- Input reaching the Mac that the user did not send, and clipboard contents leaving the device unexpectedly

Only the latest release is supported.
