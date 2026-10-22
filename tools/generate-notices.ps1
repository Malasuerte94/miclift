$ErrorActionPreference = 'Stop'
$root = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
Push-Location $root
try {
    $metadata = cargo metadata --locked --format-version 1 --filter-platform x86_64-pc-windows-msvc | ConvertFrom-Json
    if ($LASTEXITCODE -ne 0) { throw 'Cargo metadata failed' }
    $resolved = @{}; foreach ($node in $metadata.resolve.nodes) { $resolved[$node.id] = $true }
    $licenseRoot = Join-Path $root 'licenses'
    New-Item -ItemType Directory -Path $licenseRoot -Force | Out-Null
    $lines = [Collections.Generic.List[string]]::new()
    $lines.Add('# Third-party notices')
    $lines.Add('')
    $lines.Add('MicLift application code is MIT licensed. Dependencies retain their own licenses. Slint-family crates are used under the Slint Royalty-free Desktop, Mobile, and Web Applications License 2.0; About includes the required AboutSlint widget. VB-CABLE is separately licensed and is not distributed here.')
    $lines.Add('')
    $lines.Add('This inventory comes from Cargo.lock and the resolved Windows dependency graph, including build dependencies. License files are copied from the published crates. Some crates offer alternatives; inclusion of an alternative text does not mean it is the selected license.')
    $lines.Add('')
    $lines.Add('| Dependency | Version | License expression | Texts |')
    $lines.Add('| --- | --- | --- | --- |')
    $upstreamLicenseCache = @{}
    foreach ($package in ($metadata.packages | Where-Object { $_.name -ne 'miclift' -and $resolved.ContainsKey($_.id) } | Sort-Object name,version)) {
        $source = Split-Path $package.manifest_path
        $name = "$($package.name)-$($package.version)"
        $destination = Join-Path $licenseRoot $name
        New-Item -ItemType Directory -Path $destination -Force | Out-Null
        $files = @(Get-ChildItem -LiteralPath $source -File | Where-Object Name -Match '^(LICENSE|LICENCE|COPYING|NOTICE|AUTHORS|UNLICENSE|COPYRIGHT)')
        if ($package.license_file) {
            $explicit = Join-Path $source $package.license_file
            if (Test-Path -LiteralPath $explicit -PathType Leaf) { $files += Get-Item -LiteralPath $explicit }
        }
        foreach ($file in ($files | Sort-Object FullName -Unique)) { Copy-Item -LiteralPath $file.FullName -Destination $destination -Force }
        if (Test-Path -LiteralPath (Join-Path $source 'LICENSES')) { Copy-Item -LiteralPath (Join-Path $source 'LICENSES') -Destination $destination -Recurse -Force }
        if ($files.Count -eq 0) {
            # SPDX metadata stays visible; never invent a package's copyright notice.
            "$($package.name) $($package.version)`nLicense: $($package.license)`nSource: $($package.repository)`nPublished crate: https://crates.io/crates/$($package.name)/$($package.version)" | Set-Content -LiteralPath (Join-Path $destination 'SOURCE.txt') -Encoding utf8
        }
        $haveTexts = (Get-ChildItem -LiteralPath $destination -File | Where-Object Name -NE 'SOURCE.txt') -or (Test-Path -LiteralPath (Join-Path $destination 'LICENSES'))
        if (-not $haveTexts) {
            # Some workspace crates omit root license files from the published crate.
            # Retrieve text from the exact upstream commit recorded in the crate, never HEAD.
            $vcsFile = Join-Path $source '.cargo_vcs_info.json'
            if (-not (Test-Path -LiteralPath $vcsFile) -or $package.repository -notmatch '^https://github\.com/([^/]+/[^/]+)') { throw "Missing license texts for $name; add verified upstream texts before packaging" }
            $repository = $matches[1] -replace '\.git$',''
            $vcs = Get-Content -LiteralPath $vcsFile -Raw | ConvertFrom-Json
            $cacheKey = "$repository@$($vcs.git.sha1)"
            if (-not $upstreamLicenseCache.ContainsKey($cacheKey)) {
                $entries = Invoke-RestMethod -Uri "https://api.github.com/repos/$repository/contents?ref=$($vcs.git.sha1)" -Headers @{Accept='application/vnd.github+json'}
                $texts = @{}
                foreach ($entry in ($entries | Where-Object { $_.type -eq 'file' -and $_.name -match '^(LICENSE|LICENCE|COPYING|NOTICE|COPYRIGHT)' })) {
                    $texts[$entry.name] = (Invoke-WebRequest -Uri $entry.download_url).Content
                }
                $upstreamLicenseCache[$cacheKey] = $texts
            }
            $texts = $upstreamLicenseCache[$cacheKey]
            if ($texts.Count -eq 0) { throw "No upstream root license texts for $name; add verified license files manually" }
            foreach ($licenseName in $texts.Keys) { [IO.File]::WriteAllText((Join-Path $destination $licenseName), $texts[$licenseName]) }
        }
        $lines.Add("| $($package.name) | $($package.version) | $($package.license -replace '\|','\|') | [texts](licenses/$name/) |")
    }
    $lines | Set-Content -LiteralPath (Join-Path $root 'THIRD_PARTY_NOTICES.md') -Encoding utf8
} finally { Pop-Location }
