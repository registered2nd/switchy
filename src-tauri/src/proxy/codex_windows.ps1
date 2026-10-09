# Written by Switchy. Loaded by the current user's PowerShell profile.
# Each interactive window keeps its own server and terminal environment.
function global:codex {
    $swArgs = @($args)
    $swCommand = Get-Command codex -All -ErrorAction SilentlyContinue |
        Where-Object { $_.CommandType -in @('Application', 'ExternalScript') } |
        Select-Object -First 1
    if (-not $swCommand) { throw 'Codex CLI is not installed' }
    $swExecutable = $swCommand.Source
    if ($swExecutable.EndsWith('.ps1', [StringComparison]::OrdinalIgnoreCase)) {
        $swBatch = [System.IO.Path]::ChangeExtension($swExecutable, '.cmd')
        if (Test-Path -LiteralPath $swBatch) { $swExecutable = $swBatch }
    }
    $swDir = Join-Path (Split-Path $PSScriptRoot -Parent) 'switchy-sessions'
    $swBypass = @('exec', 'e', 'review', 'login', 'logout', 'mcp', 'mcp-server',
        'app-server', 'app', 'completion', 'sandbox', 'debug', 'apply', 'a',
        'cloud', 'features', 'help', 'doctor', 'plugin', 'marketplace', 'queue',
        'remote-control', 'agents', 'update', '-h', '--help', '-V', '--version')
    if (-not (Test-Path (Join-Path $swDir 'enabled')) -or
        ($swArgs.Count -gt 0 -and $swArgs[0] -in $swBypass) -or
        @($swArgs | Where-Object { $_ -in @('--no-daemon', '--add-dir', '--worktree', '--remote') -or
            $_ -match '^(--add-dir|--worktree|--remote)=' }).Count -gt 0) {
        & $swExecutable @swArgs
        return
    }

    $swResume = @()
    $swValueNext = $false
    foreach ($swArg in $swArgs) {
        if ($swValueNext) {
            $swResume += $swArg
            $swValueNext = $false
            continue
        }
        if ($swArg -eq '--') { break }
        if ($swArg -in @('-c', '--config', '-m', '--model', '-p', '--profile',
            '-s', '--sandbox', '-a', '--ask-for-approval', '-C', '--cd',
            '--local-provider', '--enable', '--disable')) {
            $swResume += $swArg
            $swValueNext = $true
        } elseif ($swArg -match '^--(config|model|profile|sandbox|ask-for-approval|cd|local-provider|enable|disable)=' -or
            $swArg -in @('--approve-for-me', '--dangerously-bypass-approvals-and-sandbox',
                '--dangerously-bypass-hook-trust', '--oss', '--search', '--no-alt-screen', '--strict-config')) {
            $swResume += $swArg
        }
    }

    [System.IO.Directory]::CreateDirectory($swDir) | Out-Null
    while ($true) {
        # Codex refuses permission flags from a remote client resuming a
        # conversation, so they go to the window's server as config.
        $swAll = $swArgs
        $swServerOpts = @()
        $swClientArgs = @()
        for ($swI = 0; $swI -lt $swArgs.Count; $swI++) {
            $swArg = $swArgs[$swI]
            $swHasValue = $swI + 1 -lt $swArgs.Count
            if ($swArg -eq '--') {
                $swClientArgs += $swArgs[$swI..($swArgs.Count - 1)]
                break
            } elseif ($swArg -in @('-c', '--config', '-m', '--model', '-p', '--profile', '-C', '--cd',
                '--local-provider', '--enable', '--disable', '-i', '--image')) {
                $swClientArgs += $swArg
                if ($swHasValue) { $swI++; $swClientArgs += $swArgs[$swI] }
            } elseif ($swArg -eq '--dangerously-bypass-approvals-and-sandbox') {
                $swServerOpts += @('-c', 'sandbox_mode=danger-full-access', '-c', 'approval_policy=never')
            } elseif ($swArg -eq '--approve-for-me') {
                $swServerOpts += @('-c', 'sandbox_mode=workspace-write', '-c', 'approval_policy=on-request',
                    '-c', 'approvals_reviewer=auto_review')
            } elseif ($swArg -in @('-s', '--sandbox')) {
                if ($swHasValue) { $swI++; $swServerOpts += @('-c', "sandbox_mode=$($swArgs[$swI])") }
            } elseif ($swArg -like '--sandbox=*') {
                $swServerOpts += @('-c', "sandbox_mode=$($swArg.Substring(10))")
            } elseif ($swArg -in @('-a', '--ask-for-approval')) {
                if ($swHasValue) { $swI++; $swServerOpts += @('-c', "approval_policy=$($swArgs[$swI])") }
            } elseif ($swArg -like '--ask-for-approval=*') {
                $swServerOpts += @('-c', "approval_policy=$($swArg.Substring(19))")
            } else {
                $swClientArgs += $swArg
            }
        }
        $swArgs = $swClientArgs
        $swId = [guid]::NewGuid().ToString('N')
        $swBase = Join-Path $swDir $swId
        $swMarker = "$swBase.session"
        $swRefresh = "$swBase.refresh"
        $swListener = [System.Net.Sockets.TcpListener]::new([System.Net.IPAddress]::Loopback, 0)
        $swListener.Start()
        $swPort = ([System.Net.IPEndPoint]$swListener.LocalEndpoint).Port
        $swListener.Stop()
        $swUrl = "ws://127.0.0.1:$swPort"
        $swBytes = New-Object byte[] 32
        $swRng = [System.Security.Cryptography.RandomNumberGenerator]::Create()
        try { $swRng.GetBytes($swBytes) } finally { $swRng.Dispose() }
        $swToken = [Convert]::ToBase64String($swBytes)
        $swHasher = [System.Security.Cryptography.SHA256]::Create()
        try { $swHash = ([BitConverter]::ToString($swHasher.ComputeHash([Text.Encoding]::UTF8.GetBytes($swToken)))).Replace('-', '').ToLowerInvariant() }
        finally { $swHasher.Dispose() }
        [System.IO.File]::WriteAllText("$swBase.token", $swToken)
        $swServer = $null
        try {
            $swServer = Start-Process -FilePath $swExecutable -ArgumentList (@('app-server') + $swServerOpts + @(
                '--listen', $swUrl, '--ws-auth', 'capability-token', '--ws-token-sha256', $swHash)) -WorkingDirectory (Get-Location).Path -WindowStyle Hidden -PassThru
            $swReady = $false
            for ($swTry = 0; $swTry -lt 150; $swTry++) {
                if ($swServer.HasExited) { break }
                try {
                    $swProbe = New-Object System.Net.Sockets.TcpClient
                    $swProbe.Connect('127.0.0.1', $swPort)
                    $swProbe.Close()
                    $swReady = $true
                    break
                } catch { Start-Sleep -Milliseconds 100 }
            }
            if (-not $swReady) {
                & $swExecutable @swAll
                return
            }
            $swVersion = (& $swExecutable --version 2>$null | Select-Object -First 1)
            $swConfigDir = if ($env:CODEX_HOME) { $env:CODEX_HOME } else { Split-Path $PSScriptRoot -Parent }
            [System.IO.File]::WriteAllText($swMarker,
                "$swUrl`nrefresh-on-exit-v1`n$((Get-Location).Path)`n$swVersion`n$swConfigDir`n$swExecutable`n")
            $env:SWITCHY_CODEX_WINDOW_TOKEN = $swToken
            try { & $swExecutable --remote $swUrl --remote-auth-token-env SWITCHY_CODEX_WINDOW_TOKEN @swArgs }
            finally { Remove-Item Env:\SWITCHY_CODEX_WINDOW_TOKEN -ErrorAction SilentlyContinue }
            $swExit = $LASTEXITCODE
            if (-not (Test-Path $swRefresh)) { return }
            $swArgs = @($swResume) + @('resume')
        } finally {
            Remove-Item -LiteralPath $swMarker, $swRefresh, "$swBase.token" -ErrorAction SilentlyContinue
            if ($swServer -and -not $swServer.HasExited) {
                & taskkill.exe /PID $swServer.Id /T /F *> $null
            }
        }
    }
}
