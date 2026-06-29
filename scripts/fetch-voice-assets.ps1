# Downloads the native Vosk runtime + small English model used for "Hey Aloe" wake-word
# spotting. These are gitignored and not checked into the repo —
# run this once after a fresh clone before `bun run tauri dev` / `tauri build`.
$ErrorActionPreference = "Stop"

$root = Split-Path -Parent $PSScriptRoot
$vendorDir = Join-Path $root "src-tauri\vendor\vosk"
$resourcesDir = Join-Path $root "src-tauri\resources"
$tmp = Join-Path $env:TEMP "aloe-voice-assets"

New-Item -ItemType Directory -Force -Path $vendorDir, $resourcesDir, $tmp | Out-Null

$isWindowsHost = $env:OS -eq "Windows_NT"
$isLinuxHost = $PSVersionTable.PSEdition -eq "Core" -and $IsLinux

if ($isWindowsHost) {
    $voskZip = Join-Path $tmp "vosk-win64.zip"
    if (-not (Test-Path (Join-Path $vendorDir "libvosk.dll"))) {
        Write-Host "Downloading libvosk native library for Windows..."
        Invoke-WebRequest -Uri "https://github.com/alphacep/vosk-api/releases/download/v0.3.45/vosk-win64-0.3.45.zip" -OutFile $voskZip
        Expand-Archive -Path $voskZip -DestinationPath $tmp -Force
        $extracted = Join-Path $tmp "vosk-win64-0.3.45"
        Copy-Item "$extracted\libvosk.dll", "$extracted\libvosk.lib", "$extracted\libgcc_s_seh-1.dll", "$extracted\libstdc++-6.dll", "$extracted\libwinpthread-1.dll", "$extracted\vosk_api.h" -Destination $vendorDir -Force
    } else {
        Write-Host "Windows libvosk already present, skipping."
    }
} elseif ($isLinuxHost) {
    $voskZip = Join-Path $tmp "vosk-linux-x86_64.zip"
    if (-not (Test-Path (Join-Path $vendorDir "libvosk.so"))) {
        Write-Host "Downloading libvosk native library for Linux..."
        Invoke-WebRequest -Uri "https://github.com/alphacep/vosk-api/releases/download/v0.3.45/vosk-linux-x86_64-0.3.45.zip" -OutFile $voskZip
        Expand-Archive -Path $voskZip -DestinationPath $tmp -Force
        $extracted = Join-Path $tmp "vosk-linux-x86_64-0.3.45"
        Copy-Item "$extracted\libvosk.so", "$extracted\vosk_api.h" -Destination $vendorDir -Force
    } else {
        Write-Host "Linux libvosk already present, skipping."
    }
} else {
    Write-Warning "No Vosk native-library download is configured for this OS. Add the matching libvosk files under $vendorDir."
}

$modelZip = Join-Path $tmp "vosk-model.zip"
$modelDir = Join-Path $resourcesDir "vosk-model-small-en-us-0.15"
if (-not (Test-Path $modelDir)) {
    Write-Host "Downloading small English Vosk model..."
    Invoke-WebRequest -Uri "https://alphacephei.com/vosk/models/vosk-model-small-en-us-0.15.zip" -OutFile $modelZip
    Expand-Archive -Path $modelZip -DestinationPath $resourcesDir -Force
} else {
    Write-Host "Vosk model already present, skipping."
}

Write-Host "Voice assets ready: $vendorDir, $modelDir"
