# Release Process for RustyCSV

## Steps

1. Update version in `mix.exs` and `native/rustycsv/Cargo.toml`
2. Update `CHANGELOG.md`
3. Run the release gates against the final release tree:
   ```bash
   cargo +nightly-2026-07-27 fmt --all --check --manifest-path native/rustycsv/Cargo.toml
   cargo +nightly-2026-07-27 clippy --manifest-path native/rustycsv/Cargo.toml --workspace --all-targets --locked -- -D warnings
   cargo +nightly-2026-07-27 clippy --manifest-path native/rustycsv/Cargo.toml --workspace --all-targets --all-features --locked -- -D warnings
   cargo +nightly-2026-07-27 test --manifest-path native/rustycsv/Cargo.toml --workspace --locked
   cargo +nightly-2026-07-27 test --manifest-path native/rustycsv/Cargo.toml --workspace --all-features --locked
   rustup component add --toolchain nightly-2026-07-27 miri rust-src
   cargo +nightly-2026-07-27 miri test --manifest-path native/rustycsv/Cargo.toml --no-default-features core::
   cargo +nightly-2026-07-27 miri test --manifest-path native/rustycsv/Cargo.toml --no-default-features strategy::encode::tests
   FORCE_RUSTYCSV_BUILD=true mix format --check-formatted
   FORCE_RUSTYCSV_BUILD=true MIX_ENV=test mix compile --force --warnings-as-errors
   FORCE_RUSTYCSV_BUILD=true MIX_ENV=test mix test
   FORCE_RUSTYCSV_BUILD=true mix credo --strict
   FORCE_RUSTYCSV_BUILD=true mix dialyzer
   FORCE_RUSTYCSV_BUILD=true mix docs --warnings-as-errors
   mix hex.audit
   cargo audit --file native/rustycsv/Cargo.lock
   package_dir="$(mktemp -d)"
   mix hex.build --unpack --output "$package_dir"
   cargo +nightly-2026-07-27 build --release --locked --manifest-path "$package_dir/native/rustycsv/Cargo.toml"
   ```
4. Review `git status --short`, stage the intended files explicitly, and commit:
   `git commit -m "Prepare vx.y.z"`
5. Push to main: `git push origin main`
6. Trigger NIF build: `gh workflow run release.yml --field version=x.y.z`
7. **Wait for ALL 45 builds to complete** (~5-10 min)
   ```bash
   gh run watch <run-id>
   ```
8. Verify draft release has 45 assets:
   ```bash
   gh release view vx.y.z --json assets --jq '.assets | length'
   ```
9. Publish draft release (assets must be public before checksums can be generated):
   ```bash
   gh release edit vx.y.z --draft=false
   ```
10. Clear checksum and generate new checksums: `rm -f checksum-Elixir.RustyCSV.Native.exs && FORCE_RUSTYCSV_BUILD=1 mix compile && mix rustler_precompiled.download RustyCSV.Native --all --print`

11. Commit checksums: `git add checksum-Elixir.RustyCSV.Native.exs && git commit -m "Add vx.y.z checksums" && git push`

12. Publish to Hex: `mix hex.publish`

## Important Notes

- **Do NOT publish the draft release (step 8) until ALL 45 jobs complete and assets are attached**
- The workflow creates a draft release - each job attaches its asset to this draft
- Publishing too early causes a race condition where later jobs fail to attach their assets
- Step 7 verifies all assets are present before proceeding
- Draft release assets are not publicly accessible, so the release must be published before generating checksums
- Publishing the release automatically creates the git tag (no need to create it manually)

## Useful Commands

```bash
# Monitor build progress
gh run list --workflow=release.yml
gh run watch <run-id>

# Check draft release assets
gh release view vx.y.z --json assets --jq '.assets | length'  # Should be 45
gh release view vx.y.z --json assets --jq '.assets[].name'

# If something goes wrong, delete and retry
gh release delete vx.y.z --yes
gh workflow run release.yml --field version=x.y.z
```
