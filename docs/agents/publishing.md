# Publishing Linux repositories

> Moved from AGENTS.md (deep dive; read when the task touches this area).

## Publishing Linux Repositories

The `scripts/publish-repo.sh` script publishes DEB and RPM packages to Cloudflare R2 (served at `repo.donutbrowser.com`). It requires Linux tools, so run it in Docker on macOS:

```bash
docker run --rm -v "$(pwd):/work" -w /work --env-file .env -e GH_TOKEN="$(gh auth token)" \
  ubuntu:24.04 bash -c '
    export DEBIAN_FRONTEND=noninteractive &&
    apt-get update -qq > /dev/null 2>&1 &&
    apt-get install -y -qq dpkg-dev createrepo-c gzip curl python3-pip > /dev/null 2>&1 &&
    pip3 install --break-system-packages awscli > /dev/null 2>&1 &&
    curl -fsSL https://cli.github.com/packages/githubcli-archive-keyring.gpg | dd of=/usr/share/keyrings/githubcli-archive-keyring.gpg 2>/dev/null &&
    echo "deb [arch=$(dpkg --print-architecture) signed-by=/usr/share/keyrings/githubcli-archive-keyring.gpg] https://cli.github.com/packages stable main" > /etc/apt/sources.list.d/github-cli.list &&
    apt-get update -qq > /dev/null 2>&1 && apt-get install -y -qq gh > /dev/null 2>&1 &&
    bash scripts/publish-repo.sh v0.18.1'
```

The `.github/workflows/publish-repos.yml` workflow runs automatically after stable releases and can also be triggered manually via `gh workflow run publish-repos.yml -f tag=v0.18.1`.

Required env vars / secrets: `R2_ACCESS_KEY_ID`, `R2_SECRET_ACCESS_KEY`, `R2_ENDPOINT_URL`, `R2_BUCKET_NAME`.
