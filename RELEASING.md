# Releasing the TypeScript tools

The two tools are independently versioned npm packages. Both start at `0.0.1`.

| Package | Executable | Release tag | Azure DevOps pipeline |
| --- | --- | --- | --- |
| `@microsoft/agent-host-protocol-cli` | `ahp` | `cli/vX.Y.Z` | [tools/ahp-cli/pipeline.yml](tools/ahp-cli/pipeline.yml) |
| `@microsoft/agent-host-protocol-conformance` | `ahp-conformance` | `conformance/vX.Y.Z` | [tools/ahp-conformance/pipeline.yml](tools/ahp-conformance/pipeline.yml) |

## One-time setup

Create an Azure DevOps pipeline for each YAML path in this repository. Use the
same `Monaco` GitHub service connection, permissions, and ESRP-backed
`microsoft/vscode-engineering` npm-package template as the TypeScript client
pipeline in `microsoft/agent-host-protocol`. Ensure the service connection can
read this samples repository and template repository, and that the publishing
identity is authorized for both npm package names.

The YAML definitions alone do not register those Azure DevOps pipelines or
grant npm publishing permissions. No npm token or GitHub Actions-to-ADO PAT
is stored in this repository.

## Release flow

1. In the relevant tool directory, update `version` in `package.json`, run
   `npm install --package-lock-only`, and add a matching `## [X.Y.Z]` release
   heading to `CHANGELOG.md`.
2. With Node 22+ for the CLI or Node 24+ for conformance, validate the package:

   ```sh
   npm ci
   npm run typecheck
   npm test
   npm pack --dry-run
   ```

   Tests include packing and installing the npm artifact outside the checkout.
   CLI builds bundle the pinned, published SDK; conformance builds compile the
   TypeScript and copy the HTML reporter assets. Neither needs a sibling
   protocol checkout. The development-only conformance examples that launch
   this repository's Rust samples still require a samples checkout and Cargo.

3. Merge the version, lockfile, and changelog changes to `main`.
4. Tag and push the package release, for example:

   ```sh
   git tag cli/v0.0.1
   git push origin cli/v0.0.1
   # Independently, for conformance:
   git tag conformance/v0.0.1
   git push origin conformance/v0.0.1
   ```

5. The tag-triggered Azure DevOps run validates the tag against the package
   version and checks for its changelog entry, then builds and tests the artifact.
6. Manually queue the matching Azure DevOps pipeline, selecting that same release
   tag, and complete its approval gates. The shared template only permits npm
   `latest` publication when `Build.Reason` is `Manual`; tag-triggered runs do
   not publish. The manual run repeats validation/build/tests before publishing
   through ESRP.

Manual Azure DevOps runs can also publish the selected branch's package version
after validation; selecting the release tag is preferred for reproducibility.
Selecting a tag from the other package is rejected. Release tags are created
explicitly in the release flow. Both pipelines set `ghCreateTag: false` and a
package-specific `ghTagPrefix`, so any template-generated GitHub release uses
the package's tag namespace rather than a bare `vX.Y.Z`.
GitHub Actions runs build/typecheck/tests for both tools on pull requests and
`main`, but never publishes.
