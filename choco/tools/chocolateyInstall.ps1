$ErrorActionPreference = 'Stop'

$toolsDir = Split-Path -Parent $MyInvocation.MyCommand.Definition
$version  = '0.2.2'
$url      = "https://github.com/pay-skill/pay-cli/releases/download/v${version}/pay-windows-amd64.exe"
$checksum = '2b9be9cdcc192c1c687a56115b2202dcf34a180ddefc354f0ebe38032a1a8142'

$packageArgs = @{
  packageName   = 'pay-cli'
  fileFullPath  = Join-Path $toolsDir 'pay.exe'
  url64bit      = $url
  checksum64    = $checksum
  checksumType64= 'sha256'
}

Get-ChocolateyWebFile @packageArgs
