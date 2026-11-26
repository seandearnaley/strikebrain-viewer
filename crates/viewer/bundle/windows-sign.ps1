# Signs one file with Azure Artifact Signing: Inno Setup's sign tool in the
# release workflow, which logs in to Azure and installs the ArtifactSigning
# module before Inno Setup runs. The account comes from the environment.
param([Parameter(Mandatory)][string]$Path)
$ErrorActionPreference = 'Stop'
Invoke-ArtifactSigning -Endpoint $env:AZURE_SIGNING_ENDPOINT `
    -CodeSigningAccountName $env:AZURE_SIGNING_ACCOUNT `
    -CertificateProfileName $env:AZURE_SIGNING_PROFILE `
    -Files $Path -FileDigest SHA256 `
    -TimestampRfc3161 'http://timestamp.acs.microsoft.com' -TimestampDigest SHA256
$signature = Get-AuthenticodeSignature -LiteralPath $Path
if ($signature.Status -ne 'Valid' -or !$signature.TimeStamperCertificate) {
    throw "No trusted, timestamped signature on $Path"
}
