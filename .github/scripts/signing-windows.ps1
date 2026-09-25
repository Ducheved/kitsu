# Import the Windows code-signing certificate when the secret exists and
# point `tauri build` at it through a config file named in
# TAURI_EXTRA_CONFIG. Without the secret this does nothing and the
# installers are unsigned.
#
#   WINDOWS_CERTIFICATE           base64 .pfx (`certutil -encode cert.pfx cert.txt`)
#   WINDOWS_CERTIFICATE_PASSWORD  its export password
#   WINDOWS_TIMESTAMP_URL         optional, default http://timestamp.digicert.com
$ErrorActionPreference = 'Stop'

if (-not $env:WINDOWS_CERTIFICATE) {
    Write-Output 'Windows: no WINDOWS_CERTIFICATE secret, installers are unsigned'
    exit 0
}
if (-not $env:WINDOWS_CERTIFICATE_PASSWORD) {
    Write-Output '::error::WINDOWS_CERTIFICATE is set but WINDOWS_CERTIFICATE_PASSWORD is not'
    exit 1
}

$dir = Join-Path $env:RUNNER_TEMP 'signing'
New-Item -ItemType Directory -Force -Path $dir | Out-Null
$b64 = Join-Path $dir 'cert.txt'
$pfx = Join-Path $dir 'cert.pfx'
Set-Content -Path $b64 -Value $env:WINDOWS_CERTIFICATE
certutil -decode $b64 $pfx | Out-Null
if ($LASTEXITCODE -ne 0) { throw 'WINDOWS_CERTIFICATE is not valid base64' }
$password = ConvertTo-SecureString -String $env:WINDOWS_CERTIFICATE_PASSWORD -Force -AsPlainText
# A .pfx can carry its chain; sign with the one that has the private key.
$cert = Import-PfxCertificate -FilePath $pfx -CertStoreLocation Cert:\CurrentUser\My -Password $password |
    Where-Object HasPrivateKey | Select-Object -First 1
if (-not $cert) { throw 'WINDOWS_CERTIFICATE has no certificate with a private key' }
Remove-Item -Force $b64, $pfx

$timestamp = if ($env:WINDOWS_TIMESTAMP_URL) { $env:WINDOWS_TIMESTAMP_URL } else { 'http://timestamp.digicert.com' }
$config = @{
    bundle = @{
        windows = @{
            certificateThumbprint = $cert.Thumbprint
            digestAlgorithm       = 'sha256'
            timestampUrl          = $timestamp
        }
    }
} | ConvertTo-Json -Depth 5
$path = Join-Path $dir 'tauri.signing.json'
Set-Content -Path $path -Value $config -Encoding utf8NoBOM
"TAURI_EXTRA_CONFIG=$path" | Out-File -FilePath $env:GITHUB_ENV -Append -Encoding utf8
Write-Output "Windows: signing with certificate $($cert.Thumbprint)"
