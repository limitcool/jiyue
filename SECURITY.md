# Security policy

## Supported versions

Jiyue is in early development and has no published release yet. Security fixes go to the `main`
branch. Once releases exist, this section will list which versions receive fixes.

## Reporting a vulnerability

Please do not open a public issue for a security problem.

Report it privately with GitHub's private vulnerability reporting: open the repository's **Security**
tab and choose **Report a vulnerability**. If that option is not visible, the repository owner has not
enabled it yet; in that case, open a public issue that says only that you have a security report and
need a private channel, without any details.

Please include:

- the version or commit you tested, and the platform (Windows, macOS or Linux);
- the steps to reproduce, and what you expected and what happened;
- the impact as you understand it (for example, data loss, exposure of note content, or exposure of a
  stored sync or AI credential).

You can expect an acknowledgement and an assessment as soon as the maintainer has time. Please give the
maintainer a reasonable period to prepare a fix before you publish details.

## Scope

In scope: the app and the MCP server (`jiyue`, `jiyue-mcp`), the sync and AI code paths, credential
storage, the installer and the release workflows.

Things the project already does by design, which are not vulnerabilities on their own: notes are plain
files, AI and sync are off by default, and API keys and sync credentials are kept in the operating
a plain-text file in your user configuration folder, readable only by your user account (see docs/configuration.md).
