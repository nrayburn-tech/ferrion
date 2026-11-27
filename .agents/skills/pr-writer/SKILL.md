---
name: pr-writer
description:
  Write or revise a Ferrion pull request title and description, with evidence of before and after
  behavior, verification results, provenance notes and the AI disclosure.
---

# Writing a Ferrion pull request

Use this skill before you open or update a PR, or when asked for a PR title. Writing a description
doesn't authorize you to push, commit or open the PR. Do those only when the human asks.

## Title

A short, imperative sentence that states the concrete behavior, in the style of the commit log:
`Keep a doc comment's hyphen rules on bytes when it reflows`, not `Update reflow.rs`. Base it on the
final diff against the base branch. A single-commit PR uses that commit's subject.

## Description

Fill in every section of `.github/pull_request_template.md`. In addition:

**Behavior fixes and features** start with the problem and an example captured by actually running
Ferrion (or the test that shows it). Never write the output by hand:

````markdown
### Example

Input (`--java-version 21`):

```java
...the minimal input...
```

Before (at the base commit):

```text
...real output, diff or error...
```

After (with this change):

```text
...real output...
```
````

Then explain the difference. For formatter changes, say what Google Java Format produces for the
same input (by running it), and name the `docs/style.md` entry for a departure. For a javac-parity
question, include what javac reports and which oracle check or difference case covers it.

**Performance changes** report before and after numbers measured the same way: same machine, same
command, several runs (`cargo benchmark run` and `compare`). State the command.

**Refactors** state what must stay identical, such as formatter output on the corpus, oracle
results or snapshot files, and how you compared it.

**Verification:** list the commands you ran (see `docs/agents/verification.md`), including any
corpus or oracle checks. Say plainly which checks you skipped and why.

**Provenance:** fill in the template's provenance notes. Name any upstream implementation you have
studied, and any copied code with its source and license. Write "None" if there is none.

**AI disclosure:** the exact model name and version, the reasoning effort, and the tier. If you
don't know the exact model or effort, say so; don't guess.
