<#
.SYNOPSIS
  Builds WEEP for Windows and packs the release zip.

.DESCRIPTION
  pwsh tools/windows/package.ps1 [-SkipBuild] [-OutDir dist]

  Runs `cargo build --release --locked -p weep` (into $env:CARGO_TARGET_DIR when set: on a
  network share point it at a local disk), checks the binary is what a release needs, then
  writes <OutDir>/weep-<version>-windows-<arch>.zip and its .sha256:

    weep-<version>-windows-<arch>/
      weep.exe                 icons, fonts, themes and keymap are compiled in
      LICENSE.txt              WEEP (MIT)
      licenses/                what the binary bundles: Inter and JetBrains Mono (OFL),
                               JetBrains' expui icons (Apache-2.0)

  The checks: a GUI-subsystem executable (no console window from Explorer), no
  VCRUNTIME/MSVCP import (the C runtime is static, see .cargo/config.toml), and the
  version resource that comes with the icon. git, the shells and rust-analyzer are found on PATH at run time.
#>
param(
    [switch]$SkipBuild,
    [string]$OutDir = "dist"
)
$ErrorActionPreference = "Stop"
$repo = Resolve-Path (Join-Path $PSScriptRoot "../..")
Push-Location $repo
try {
    $meta = cargo metadata --no-deps --format-version 1 | ConvertFrom-Json
    $version = ($meta.packages | Where-Object name -eq "weep").version
    $targetDir = $meta.target_directory
    $arch = switch ($env:PROCESSOR_ARCHITECTURE) { "ARM64" { "arm64" } default { "x64" } }

    if (-not $SkipBuild) {
        cargo build --release --locked -p weep
        if ($LASTEXITCODE -ne 0) { throw "cargo build failed ($LASTEXITCODE)" }
    }
    $exe = Join-Path $targetDir "release/weep.exe"
    if (-not (Test-Path $exe)) { throw "no $exe; build first or drop -SkipBuild" }

    # The release checks, read straight off the PE file.
    $bytes = [IO.File]::ReadAllBytes($exe)
    $pe = [BitConverter]::ToInt32($bytes, 0x3C)
    $subsystem = [BitConverter]::ToUInt16($bytes, $pe + 24 + 68)
    if ($subsystem -ne 2) { throw "weep.exe is subsystem $subsystem, not GUI (2): was it a debug build?" }
    $text = [Text.Encoding]::ASCII.GetString($bytes)
    $crt = [regex]::Matches($text, '(?i)\b(vcruntime\d+(_1)?|msvcp\d+)\.dll') | ForEach-Object Value | Sort-Object -Unique
    if ($crt) { throw "weep.exe imports $($crt -join ', '): the static CRT flag did not apply (is RUSTFLAGS set?)" }
    # The icon and the version block come from the same resource script (build.rs).
    $info = (Get-Item $exe).VersionInfo
    if ($info.ProductName -ne "WEEP" -or $info.ProductVersion -ne $version) {
        throw "weep.exe has no WEEP $version version resource: build.rs's resources did not link"
    }

    $name = "weep-$version-windows-$arch"
    $stage = Join-Path $OutDir $name
    if (Test-Path $stage) { Remove-Item -Recurse -Force $stage }
    New-Item -ItemType Directory -Force (Join-Path $stage "licenses") | Out-Null

    Copy-Item $exe $stage
    Copy-Item LICENSE (Join-Path $stage "LICENSE.txt")
    $assets = "crates/weep/assets"
    Copy-Item "$assets/fonts/LICENSE-Inter.txt" (Join-Path $stage "licenses/Inter-OFL.txt")
    Copy-Item "$assets/fonts/LICENSE-JetBrainsMono.txt" (Join-Path $stage "licenses/JetBrainsMono-OFL.txt")
    Copy-Item "$assets/icons/expui/LICENSE.txt" (Join-Path $stage "licenses/expui-icons-NOTICE.txt")
    Copy-Item "$assets/icons/expui/LICENSE-Apache-2.0.txt" (Join-Path $stage "licenses/Apache-2.0.txt")

    $zip = Join-Path $OutDir "$name.zip"
    if (Test-Path $zip) { Remove-Item -Force $zip }
    Compress-Archive -Path $stage -DestinationPath $zip
    $hash = (Get-FileHash -Algorithm SHA256 $zip).Hash.ToLower()
    "$hash  $name.zip" | Set-Content -NoNewline -Encoding ascii "$zip.sha256"

    $zipItem = Get-Item $zip
    Write-Host "$($zipItem.FullName)  ($([math]::Round($zipItem.Length / 1MB, 1)) MB)"
    Write-Host "sha256 $hash"
}
finally {
    Pop-Location
}
