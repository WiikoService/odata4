# Builds the OData4Native add-in for every target whose toolchain is present, checks the libraries
# (devcli addin-check: Native API exports, no MinGW runtime DLLs, GLIBC <= 2.17) and packs a ZIP with
# manifest.xml into src/cfe/CommonTemplates/OData4_*/Ext/Template.bin. Missing toolchains are reported as
# SKIP and left out of the manifest. -RequireAll turns any SKIP into an error (release builds).
param([switch]$RequireAll)
$ErrorActionPreference = "Stop"
[Console]::OutputEncoding = [Text.Encoding]::UTF8
$native = $PSScriptRoot
$repo = (Resolve-Path (Join-Path $native "..")).Path
$installed = @(& rustup target list --installed)
$mingw32 = "C:\msys64\mingw32\bin\i686-w64-mingw32-gcc.exe"
$targets = @(
    @{ Triple = "x86_64-pc-windows-gnu"; Os = "Windows"; Arch = "x86_64"; File = "odata4_native.dll"; Name = "odata4_native_x64.dll"; Zig = $false },
    @{ Triple = "i686-pc-windows-gnu"; Os = "Windows"; Arch = "i386"; File = "odata4_native.dll"; Name = "odata4_native_x86.dll"; Zig = $false },
    @{ Triple = "x86_64-unknown-linux-gnu"; Os = "Linux"; Arch = "x86_64"; File = "libodata4_native.so"; Name = "libodata4_native_x64.so"; Zig = $true },
    @{ Triple = "i686-unknown-linux-gnu"; Os = "Linux"; Arch = "i386"; File = "libodata4_native.so"; Name = "libodata4_native_x86.so"; Zig = $true }
)
$built = @()
$skipped = @()
foreach ($t in $targets) {
    $reason = $null
    if ($installed -notcontains $t.Triple) {
        $reason = "rustup target not installed"
    } elseif ($t.Triple -eq "i686-pc-windows-gnu" -and -not (Test-Path $mingw32)) {
        $reason = "linker $mingw32 not found"
    } elseif ($t.Zig -and (-not (Get-Command zig -ErrorAction SilentlyContinue) -or -not (Get-Command cargo-zigbuild -ErrorAction SilentlyContinue))) {
        $reason = "zig or cargo-zigbuild not on PATH"
    }
    if ($reason) {
        Write-Output "SKIP $($t.Triple): $reason"
        $skipped += $t.Triple
        continue
    }
    Push-Location $native
    try {
        if ($t.Triple -eq "i686-pc-windows-gnu") {
            $env:CARGO_TARGET_I686_PC_WINDOWS_GNU_LINKER = $mingw32
            # The gcc driver needs its own bin directory on PATH: without it, it cannot find ld/collect2
            # and fails with an empty linker message.
            $bin = Split-Path $mingw32
            if (($env:PATH -split ';') -notcontains $bin) { $env:PATH = $bin + ';' + $env:PATH }
        }
        if ($t.Zig) {
            & cargo zigbuild --release -p odata4_addin --target "$($t.Triple).2.17"
        } else {
            & cargo build --release -p odata4_addin --target $t.Triple
        }
        if ($LASTEXITCODE -ne 0) { throw "build $($t.Triple): exit $LASTEXITCODE" }
    } finally {
        Pop-Location
    }
    $t.Path = Join-Path $native "target\$($t.Triple)\release\$($t.File)"
    if (-not (Test-Path $t.Path)) { throw "no build output $($t.Path)" }
    $built += $t
}
if (-not ($built | Where-Object { $_.Triple -eq "x86_64-pc-windows-gnu" })) { throw "x86_64-pc-windows-gnu was not built" }
if ($RequireAll -and $skipped.Count -gt 0) { throw ("skipped targets: " + ($skipped -join ", ")) }
# The public repository ships without tools/devcli: there the check is skipped.
if (Test-Path (Join-Path $repo "tools\devcli")) {
    Push-Location $repo
    try {
        & go run ./tools/devcli addin-check --glibc 2.17 @($built | ForEach-Object { $_.Path })
        if ($LASTEXITCODE -ne 0) { throw "addin-check: exit $LASTEXITCODE" }
    } finally {
        Pop-Location
    }
} else {
    Write-Output "SKIP addin-check: tools\devcli not present"
}
$stage = Join-Path $env:TEMP "odata4_native_pack"
if (Test-Path $stage) { Remove-Item $stage -Recurse -Force }
New-Item -ItemType Directory $stage | Out-Null
$lines = @('<?xml version="1.0" encoding="UTF-8"?>', '<bundle xmlns="http://v8.1c.ru/8.2/addin/bundle">')
foreach ($t in $built) {
    Copy-Item $t.Path (Join-Path $stage $t.Name)
    $lines += "`t<component os=`"$($t.Os)`" path=`"$($t.Name)`" type=`"native`" arch=`"$($t.Arch)`"/>"
}
$lines += '</bundle>'
[IO.File]::WriteAllText((Join-Path $stage "manifest.xml"), (($lines -join "`n") + "`n"), (New-Object System.Text.UTF8Encoding $false))
$zip = Join-Path $env:TEMP "odata4_native.zip"
if (Test-Path $zip) { Remove-Item $zip -Force }
Compress-Archive -Path (Join-Path $stage "*") -DestinationPath $zip
$template = Get-ChildItem (Join-Path $repo "src\cfe\CommonTemplates") -Directory -Filter "OData4_*" | Select-Object -First 1
if (-not $template) { throw "no src\cfe\CommonTemplates\OData4_* directory" }
$dest = Join-Path $template.FullName "Ext\Template.bin"
Copy-Item $zip $dest -Force
Write-Output ("built: " + (($built | ForEach-Object { $_.Triple }) -join ", "))
Write-Output ("skipped: " + ($skipped -join ", "))
Write-Output ("Template.bin: {0} bytes" -f (Get-Item $dest).Length)
