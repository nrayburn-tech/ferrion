# Security policy

Ferrion reads source files, class files, jars and JDK module images that it doesn't control, in a
command-line tool, a build plugin and a long-running language server. It also rewrites source
files. We treat bugs that let crafted input crash, hang or exhaust it, or that make it write files
it shouldn't, as security issues.

## Reporting a vulnerability

Report vulnerabilities privately through
[GitHub private vulnerability reporting](https://github.com/nrayburn-tech/ferrion/security/advisories/new).
Please don't open a public issue or PR.

Include the affected version or commit, the platform, reproduction steps, and the impact you see. We
aim to acknowledge reports within 7 days.

## In scope (examples)

- A crafted Java file, class file, jar, module image or `ct.sym` that makes Ferrion panic, hang,
  overflow the stack or allocate without bound
- Writing outside the files a run covers, for example by following a symlink
- Any network request made by Ferrion
- Command or argument injection in the Maven plugin or IntelliJ extension

## Not security issues

Formatting that changes a program's meaning, or a wrong finding, is a serious bug, but not a
vulnerability. Report it as an ordinary issue.

## Supported versions

Ferrion is not released yet. Only the latest commit on `main` receives fixes.
