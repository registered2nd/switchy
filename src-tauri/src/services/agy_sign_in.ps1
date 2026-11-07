# Written by Switchy while "Open sign-in links manually" is on. Loaded by the
# current user's PowerShell profile.
# agy opens no browser for its sign-in when it believes it runs over SSH
# (SSH_CLIENT, SSH_TTY or SSH_CONNECTION set) and shows the link instead. The
# variable also changes agy's terminal-feature guesses, so it is set only for
# an agy started with no saved sign-in, which is the session that signs in.
function global:agy {
    $swCommand = Get-Command agy -CommandType Application -ErrorAction SilentlyContinue |
        Select-Object -First 1
    if (-not $swCommand) { throw 'Antigravity CLI is not installed' }
    # cmdkey names the target in its header whether or not the entry exists,
    # and again on the entry's own line when it does.
    $swSignedIn = @(cmdkey "/list:gemini:antigravity" 2>$null |
        Select-String -SimpleMatch 'gemini:antigravity').Count -ge 2
    if ($swSignedIn -or $env:SSH_CLIENT -or $env:SSH_TTY -or $env:SSH_CONNECTION) {
        & $swCommand.Source @args
        return
    }
    $env:SSH_CLIENT = '127.0.0.1 0 0'
    try {
        & $swCommand.Source @args
    } finally {
        Remove-Item Env:SSH_CLIENT -ErrorAction SilentlyContinue
    }
}
