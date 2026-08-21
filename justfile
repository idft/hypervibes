test:
    cargo test --workspace
    python agent-runtime/test_coding_validate.py
    python agent-runtime/mcp/test_server.py
    pnpm test:js

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
