# p11k engine bootstrap (pwsh) -- protocol layer + user config.
$P11kAnnounce = $env:P11K_ANNOUNCE
$P11kAck = $env:P11K_ACK
# Last prompt shape from the engine ack; reused when the engine does not answer (ack timeout).
$script:P11kHeaderRows = 1
$script:P11kPrefix = $env:P11K_PREFIX_OK
$script:P11kAckWarned = $false
$script:P11kCols = $Host.UI.RawUI.WindowSize.Width
$script:P11kRows = $Host.UI.RawUI.WindowSize.Height

function global:P11kEnginePrompt {
    # $? must be read on the first line: any later statement in prompt overwrites it.
    $P11kCode = if ($?) { 0 } elseif ($global:LASTEXITCODE -is [int]) { $global:LASTEXITCODE } else { 1 }
    $P11kSize = $Host.UI.RawUI.WindowSize
    if ($P11kSize.Width -ne $script:P11kCols -or $P11kSize.Height -ne $script:P11kRows) {
        $script:P11kCols = $P11kSize.Width
        $script:P11kRows = $P11kSize.Height
        # Announcing only `r` and returning would leave the old prompt on screen: the engine
        # may not be in prompt state right now (just pressed enter), so it would not repaint.
        # Hence fall through to the normal announce.
        [IO.File]::AppendAllText($script:P11kAnnounce, "r`n")
    }
    $P11kJobs = @(Get-Job -ErrorAction SilentlyContinue).Count
    # Send the command number, not the history count: zsh reports HISTCMD, and
    # HistoryInfo.Id is PowerShell's nearest equivalent (session-unique and
    # monotonic). .Count drops when MaximumHistoryCount trims; Id survives that.
    # After Clear-History there is nothing left to read, so it reports 0.
    $P11kLast = Get-History -Count 1
    $P11kHist = if ($P11kLast) { $P11kLast.Id } else { 0 }
    # PowerShell records the command start and end on the history entry. Send the duration in
    # milliseconds. The engine then does not infer it from the Enter → precmd wall clock.
    $P11kExecMs = if ($P11kLast.StartExecutionTime -and $P11kLast.EndExecutionTime) {
        [int]($P11kLast.EndExecutionTime - $P11kLast.StartExecutionTime).TotalMilliseconds
    } else { '' }
    # Escape the cwd for the tab-separated announce line: a directory name can contain a tab,
    # a newline or a backslash, which would split or merge fields.
    $P11kPwdEsc = $PWD.Path.Replace('\', '\\').Replace("`t", '\t').Replace("`n", '\n').Replace("`r", '\r')
    [IO.File]::AppendAllText($script:P11kAnnounce, "h`t$P11kCode`t$P11kPwdEsc`t$P11kJobs`t$P11kHist`t$P11kExecMs`n")
    # Wait at most two seconds for the ack. A dead or stuck engine must not block the prompt;
    # the previous prompt shape is reused and the fallback is reported once.
    $P11kWait = 0
    while (-not [IO.File]::Exists($script:P11kAck)) {
        Start-Sleep -Milliseconds 5
        $P11kWait++
        if ($P11kWait -ge 400) { break }
    }
    if ([IO.File]::Exists($script:P11kAck)) {
        # The ack carries the prompt shape: header height on the first line, then the two prefix
        # strings. Reading them every prompt is what lets a reload change either without a restart.
        $P11kAckLines = @(Get-Content $script:P11kAck)
        [IO.File]::Delete($script:P11kAck)
        $script:P11kHeaderRows = [int]$P11kAckLines[0]
        $script:P11kPrefix = $P11kAckLines[1]
    } elseif (-not $script:P11kAckWarned) {
        $script:P11kAckWarned = $true
        if ($P11kCode -eq 0) {
            $script:P11kPrefix = $env:P11K_PREFIX_OK
        } else {
            $script:P11kPrefix = $env:P11K_PREFIX_ERR
        }
        [Console]::Error.WriteLine("p11k: the engine did not answer in time; the prompt is drawn without a header")
    }
    # Prompt is ready, clear the anti-recursion flag (same as zsh's zle-line-init): a manual
    # p11k started later in the session is then not treated as a recursive load.
    if ($null -ne $env:P11K_ENGINE) { $env:P11K_ENGINE = $null }
    # The prefix belongs in the prompt string: PSReadLine draws the buffer right after it.
    # Every prompt prints its own header rows. On the first prompt the instant header stays
    # above as history instead of being overwritten.
    if ($env:P11K_INSTANT) {
        Remove-Item Env:P11K_INSTANT -ErrorAction SilentlyContinue
    }
    $P11kLead = "`n" * $script:P11kHeaderRows
    "$P11kLead`e]1337;p11k;p`a$($script:P11kPrefix)"
}

function global:prompt { P11kEnginePrompt }

# ===== user config =====
# Start the user profile on a fresh line. During the instant phase the prefix and the typed text
# are on the current line; without this break, the profile output would follow the typed text.
if ($env:P11K_INSTANT) {
    [Console]::Out.Write("`r`n")
}
$P11kUserProfile = if ($env:P11K_USER_PROFILE) { $env:P11K_USER_PROFILE } else { $PROFILE }
if ($P11kUserProfile -and (Test-Path -LiteralPath $P11kUserProfile)) {
    # A failed dot-source must not break the later protocol invariants that re-declare prompt.
    try { . $P11kUserProfile } catch { Write-Warning "p11k: cannot load $P11kUserProfile`: $_" }
}

# ===== protocol invariants =====
# The user profile may replace prompt; point it back to the engine implementation here.
$P11kAnnounce = $env:P11K_ANNOUNCE
$P11kAck = $env:P11K_ACK
function global:prompt { P11kEnginePrompt }
