test:
    cargo test --workspace
    python agent-runtime/test_coding_validate.py
    python agent-runtime/mcp/test_server.py
    pnpm test:js

release version:
    #!/usr/bin/env bash
    set -euo pipefail

    version="{{version}}"
    tag="v${version}"

    if [[ ! "${version}" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
        printf 'Release version must be X.Y.Z: %s\n' "${version}" >&2
        exit 1
    fi

    if [[ -n "$(git status --short)" ]]; then
        printf '%s\n' 'Release preparation requires a clean worktree.' >&2
        exit 1
    fi

    branch="$(git branch --show-current)"
    if [[ "${branch}" != "master" ]]; then
        printf 'Release preparation must run from master, not %s.\n' "${branch}" >&2
        exit 1
    fi

    if git rev-parse --verify --quiet "refs/tags/${tag}" >/dev/null \
        || git ls-remote --exit-code --tags origin "refs/tags/${tag}" >/dev/null 2>&1; then
        printf 'Tag already exists: %s\n' "${tag}" >&2
        exit 1
    fi

    current_version="$(sed -nE '0,/^version = "[0-9]+\.[0-9]+\.[0-9]+"/{s/^version = "([0-9]+\.[0-9]+\.[0-9]+)"/\1/p;}' Cargo.toml)"
    if [[ -z "${current_version}" ]]; then
        printf '%s\n' 'Could not determine the application version from Cargo.toml.' >&2
        exit 1
    fi
    if [[ "${current_version}" == "${version}" ]]; then
        printf 'Cargo.toml is already at version %s.\n' "${version}" >&2
        exit 1
    fi

    sed -i -E "0,/^version = \"[0-9]+\.[0-9]+\.[0-9]+\"/{s//version = \"${version}\"/;}" Cargo.toml
    npm pkg set "version=${version}" >/dev/null
    sed -i -E "s#(ghcr.io/idft/hypervibes:)[^[:space:]]+#\1${version}#g" podman-compose.yaml

    # Cargo updates the root package entry in Cargo.lock without changing the
    # dependency resolution.
    cargo check --workspace

    cargo fmt --check
    cargo clippy --workspace --all-targets -- -D warnings
    just test

    cargo_version="$(
        cargo metadata --no-deps --format-version=1 --locked \
            | node -e 'let data = ""; process.stdin.on("data", (chunk) => data += chunk); process.stdin.on("end", () => { const pkg = JSON.parse(data).packages.find((item) => item.name === "hypervibes"); process.stdout.write(pkg.version); });'
    )"
    package_version="$(node -p 'require("./package.json").version')"

    if [[ "${cargo_version}" != "${version}" || "${package_version}" != "${version}" ]]; then
        printf 'Version mismatch after release preparation.\n' >&2
        exit 1
    fi

    printf '\nRelease diff:\n'
    git diff -- Cargo.toml Cargo.lock package.json podman-compose.yaml
    printf '\nCreate the release commit and annotated tag %s locally? [y/N] ' "${tag}"
    read -r confirm
    if [[ "${confirm}" != "y" && "${confirm}" != "Y" ]]; then
        printf '%s\n' 'Release files left uncommitted.'
        exit 0
    fi

    git add -- Cargo.toml Cargo.lock package.json podman-compose.yaml
    git commit -m "prepare ${tag} release"
    git tag -a "${tag}" -m "${tag}"

    printf '\nCreated the release commit and tag locally. Push both to origin now? [y/N] '
    read -r push_confirm
    if [[ "${push_confirm}" == "y" || "${push_confirm}" == "Y" ]]; then
        git push origin HEAD --follow-tags
    else
        printf 'Release commit and tag remain local. Push later with:\n'
        printf '  git push origin HEAD --follow-tags\n'
    fi

dev:
    podman-compose -f podman-compose.dev.yaml up

website:
    pnpm --dir website start --host 0.0.0.0

branding:
    #!/usr/bin/env bash
    set -euo pipefail

    source="assets/branding/logo.png"
    static_dir="static"
    website_image_dir="website/static/img"

    if ! command -v magick >/dev/null 2>&1; then
        printf '%s\n' "ImageMagick is required. Install it and retry." >&2
        exit 1
    fi

    if [[ ! -f "${source}" ]]; then
        printf 'Branding source not found: %s\n' "${source}" >&2
        exit 1
    fi

    mkdir -p "${website_image_dir}"

    generate_png() {
        local size="$1"
        local output="$2"

        magick "${source}" \
            -background none \
            -resize "${size}x${size}" \
            -gravity center \
            -extent "${size}x${size}" \
            -strip \
            "${output}"
    }

    generate_png 16 "${static_dir}/favicon-16x16.png"
    generate_png 32 "${static_dir}/favicon-32x32.png"
    generate_png 180 "${static_dir}/apple-touch-icon.png"
    generate_png 192 "${static_dir}/android-chrome-192x192.png"
    generate_png 512 "${static_dir}/android-chrome-512x512.png"
    generate_png 64 "${website_image_dir}/favicon.png"
    generate_png 512 "${website_image_dir}/logo.png"

    magick "${source}" \
        -background none \
        -define icon:auto-resize=16,32,48 \
        -strip \
        "${static_dir}/favicon.ico"

    printf 'Generated branding assets from %s\n' "${source}"
