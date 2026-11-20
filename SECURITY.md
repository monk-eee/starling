# Security

Please do not publish vulnerability details, credentials, workload files,
real service addresses or raw reports in a GitHub issue or pull request.

Check the repository's [Security page](https://github.com/monk-eee/starling/security)
for a private **Report a vulnerability** option. If it is unavailable,
[open an issue](https://github.com/monk-eee/starling/issues/new) asking the
maintainer for a private reporting channel. Include **no technical details
or attachments** in that public request. Once a private channel is agreed,
include the affected commit or version, expected and observed behavior, and
impact. Use synthetic data wherever possible.

Test only local MCP services you control. The browser dashboard, the MCP
target, and the bird telemetry listener serve different purposes. Do not
expose the browser as a substitute for the
[mutually authenticated telemetry listener](developer.md#connect-across-machines).
Treat run, tool, task, prompt and endpoint alias names as public: redaction
of payloads does not remove a secret a user has placed in a label.
