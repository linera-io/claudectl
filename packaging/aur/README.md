# AUR Packaging

This directory contains repo-side templates for the `claudectl-bin` AUR package.

The actual AUR package must live in its own AUR git repository, but these files
keep the package definition reproducible from the main repo.

## Update flow

1. Fetch the x86_64 Linux release digest for the new tag.
2. Compute the SHA256 of `LICENSE`.
3. Re-render the package files:

```bash
./scripts/render-aur-bin-files.sh <version> <linux_x86_64_sha256> <license_sha256> packaging/aur/claudectl-bin
```

4. Copy `packaging/aur/claudectl-bin/PKGBUILD` and `.SRCINFO` into the AUR repo.
5. Commit and push the AUR repo update.
