param(
    [Parameter(Mandatory = $true)]
    [string]$ExtractDir,
    [Parameter(Mandatory = $true)]
    [string]$DllDestination
)

$ErrorActionPreference = "Stop"

function Convert-RvaToOffset([byte[]]$Bytes, [int]$Pe, [uint32]$Rva) {
    $sectionCount = [BitConverter]::ToUInt16($Bytes, $Pe + 6)
    $optionalSize = [BitConverter]::ToUInt16($Bytes, $Pe + 20)
    $sectionStart = $Pe + 24 + $optionalSize
    for ($index = 0; $index -lt $sectionCount; $index++) {
        $section = $sectionStart + ($index * 40)
        $virtualAddress = [BitConverter]::ToUInt32($Bytes, $section + 12)
        $virtualSize = [BitConverter]::ToUInt32($Bytes, $section + 8)
        $rawSize = [BitConverter]::ToUInt32($Bytes, $section + 16)
        $rawPtr = [BitConverter]::ToUInt32($Bytes, $section + 20)
        $span = [Math]::Max($virtualSize, $rawSize)
        if ($Rva -ge $virtualAddress -and $Rva -lt ($virtualAddress + $span)) {
            return [int]($Rva - $virtualAddress + $rawPtr)
        }
    }
    throw "Could not map RVA $Rva to a file offset"
}

function Read-CString([byte[]]$Bytes, [int]$Offset) {
    $end = $Offset
    while ($end -lt $Bytes.Length -and $Bytes[$end] -ne 0) {
        $end++
    }
    return [System.Text.Encoding]::ASCII.GetString($Bytes, $Offset, $end - $Offset)
}

function Get-PeExports([string]$DllPath) {
    $bytes = [System.IO.File]::ReadAllBytes($DllPath)
    if ($bytes.Length -lt 64 -or [BitConverter]::ToUInt16($bytes, 0) -ne 0x5A4D) {
        throw "$DllPath is not a PE file"
    }
    $pe = [BitConverter]::ToInt32($bytes, 0x3C)
    if ([BitConverter]::ToUInt32($bytes, $pe) -ne 0x4550) {
        throw "$DllPath is missing a PE signature"
    }
    $optional = $pe + 24
    $magic = [BitConverter]::ToUInt16($bytes, $optional)
    $exportDir = if ($magic -eq 0x20B) { $optional + 112 } else { $optional + 96 }
    $exportRva = [BitConverter]::ToUInt32($bytes, $exportDir)
    if ($exportRva -eq 0) {
        throw "$DllPath has no export table"
    }
    $exportOffset = Convert-RvaToOffset $bytes $pe $exportRva
    $nameCount = [BitConverter]::ToUInt32($bytes, $exportOffset + 24)
    $namesRva = [BitConverter]::ToUInt32($bytes, $exportOffset + 32)
    $namesOffset = Convert-RvaToOffset $bytes $pe $namesRva
    $names = New-Object System.Collections.Generic.List[string]
    for ($index = 0; $index -lt $nameCount; $index++) {
        $nameRva = [BitConverter]::ToUInt32($bytes, $namesOffset + ($index * 4))
        $nameOffset = Convert-RvaToOffset $bytes $pe $nameRva
        $name = Read-CString $bytes $nameOffset
        if ($name) {
            [void]$names.Add($name)
        }
    }
    if ($names.Count -eq 0) {
        throw "$DllPath exported no named symbols"
    }
    return $names
}

function Get-LibExe {
    $vswhere = Join-Path ${env:ProgramFiles(x86)} "Microsoft Visual Studio\Installer\vswhere.exe"
    if (-not (Test-Path $vswhere)) {
        throw "vswhere.exe was not found"
    }
    $vs = & $vswhere -latest -products * -property installationPath
    if (-not $vs) {
        throw "Visual Studio was not found"
    }
    $lib = Get-ChildItem -Path (Join-Path $vs "VC\Tools\MSVC") -Recurse -Filter "lib.exe" |
        Where-Object { $_.FullName -match '\\Hostx64\\x64\\lib.exe$' } |
        Sort-Object FullName -Descending |
        Select-Object -First 1
    if (-not $lib) {
        throw "lib.exe was not found"
    }
    return $lib.FullName
}

$dll = Get-ChildItem -Path $ExtractDir -Recurse -File |
    Where-Object { $_.Name -in @("mpv-2.dll", "libmpv-2.dll") } |
    Select-Object -First 1
if (-not $dll) {
    throw "mpv-2.dll was not in the libmpv archive"
}

New-Item -ItemType Directory -Force (Split-Path $DllDestination) | Out-Null
Copy-Item $dll.FullName $DllDestination -Force

$libDir = $dll.DirectoryName
$exports = Get-PeExports $dll.FullName
$defPath = Join-Path $libDir "mpv.def"
$lines = @("LIBRARY mpv-2.dll", "EXPORTS") + ($exports | ForEach-Object { "    $_" })
Set-Content -Path $defPath -Value $lines -Encoding ascii

$libExe = Get-LibExe
$libPath = Join-Path $libDir "mpv.lib"
& $libExe "/NOLOGO" "/MACHINE:X64" "/DEF:$defPath" "/OUT:$libPath" "/NAME:mpv-2.dll"
if ($LASTEXITCODE -ne 0 -or -not (Test-Path $libPath)) {
    throw "lib.exe failed to create mpv.lib"
}

Write-Host "Using $($dll.FullName)"
Write-Host "Created $libPath ($($exports.Count) exports)"
"MPV_LIB_DIR=$libDir" | Out-File -FilePath $env:GITHUB_ENV -Append
