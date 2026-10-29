# Shared CI

Moved to [sapphire-turtle/.github](https://github.com/sapphire-turtle/.github): the reusable
workflows (call them as `sapphire-turtle/.github/.github/workflows/<file>@v1`), the caller
workflow templates, the repository templates and the conventions.

`.github/workflows/shared-*.yml` stay here only until every repository that calls them at
`@main` has moved (#352).
