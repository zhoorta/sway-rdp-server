# Notes for Claude

## Commits

- Every commit is authored and committed by the repository's owner, never by Claude. If git's identity in the clone is Claude's, ask the owner for theirs before committing.
- Add no `Co-Authored-By: Claude`, `Claude-Session` or other Claude attribution lines to commit messages, and none to pull request descriptions.

## The site

- The documentation pages are built from `site/pages/` with `python3 site/build.py`; the generated files are committed, so run it after editing a page and commit both.
- `sitemap.xml` takes its dates from git history: in a shallow clone, fetch more history first (`git fetch --depth=1000 origin main`), and run the build again after committing.
