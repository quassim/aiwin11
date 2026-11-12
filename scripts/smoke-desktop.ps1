<# Runs a release smoke test through Windows UI Automation. -AllowCloud permits
   sending the synthetic prompt to the provider already selected in AiWin11. #>
param([switch]$AllowCloud, [switch]$InspectOnly, [switch]$ModelsOnly, [switch]$PaletteOnly)

$ErrorActionPreference = 'Stop'
$exe = Join-Path (Split-Path -Parent $PSScriptRoot) 'target\release\aiwin11-desktop.exe'
if (!(Test-Path -LiteralPath $exe -PathType Leaf)) { throw 'Build the release application first.' }
if (Get-Process -Name aiwin11-desktop, AiWin11 -ErrorAction SilentlyContinue) { throw 'Close AiWin11 before testing.' }
Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes
$db = Join-Path $env:LOCALAPPDATA 'app.aiwin11.desktop\aiwin11.sqlite3'
$python = (Get-Command python.exe -CommandType Application | Select-Object -First 1).Source

function Read-TaskEvents([long]$after) {
    $query = @'
import json, sqlite3, sys
from pathlib import Path
with sqlite3.connect(Path(sys.argv[1]).resolve().as_uri() + '?mode=ro', uri=True) as db:
    last = db.execute('SELECT COALESCE(MAX(sequence), 0) FROM events').fetchone()[0]
    task = db.execute('SELECT task_id FROM events WHERE kind = ? AND sequence > ? ORDER BY sequence LIMIT 1',
                      ('task.created', int(sys.argv[2]))).fetchone()
    kinds = [row[0] for row in db.execute('SELECT kind FROM events WHERE task_id = ? ORDER BY sequence', task)] if task else []
    display = db.execute('SELECT EXISTS(SELECT 1 FROM events AS broker JOIN events AS desktop '
                         'ON desktop.task_id = broker.task_id AND desktop.session_id = broker.session_id '
                         'WHERE broker.sequence > ? AND broker.kind = ? AND desktop.kind = ?)',
                         (int(sys.argv[2]), 'broker.windows.completed', 'displays.completed')).fetchone()[0]
    speech = db.execute('SELECT EXISTS(SELECT 1 FROM events WHERE sequence > ? AND kind = ?)',
                        (int(sys.argv[2]), 'speech.list.completed')).fetchone()[0]
    palette = [int(code[3:]) for (code,) in db.execute(
        'SELECT code FROM events WHERE sequence > ? AND kind = ? ORDER BY sequence', (int(sys.argv[2]), 'ui.palette.ready'))]
print(json.dumps({'last': last, 'kinds': kinds, 'brokerDisplay': bool(display), 'speechCatalog': bool(speech), 'palette': palette}))
'@
    $result = & $python -B -c $query $db $after
    if ($LASTEXITCODE -ne 0) { throw 'Cannot read the test task audit.' }
    return $result | ConvertFrom-Json
}

function Wait-For([scriptblock]$check, [string]$failure, [int]$seconds = 15) {
    $watch = [Diagnostics.Stopwatch]::StartNew()
    do {
        $result = & $check
        if ($result) { return $result }
        Start-Sleep -Milliseconds 120
    } while ($watch.Elapsed.TotalSeconds -lt $seconds)
    throw $failure
}

function Find-Control($window, [string]$name, $type) {
    $named = [System.Windows.Automation.PropertyCondition]::new(
        [System.Windows.Automation.AutomationElement]::NameProperty, $name)
    $typed = [System.Windows.Automation.PropertyCondition]::new(
        [System.Windows.Automation.AutomationElement]::ControlTypeProperty, $type)
    return $window.FindFirst([System.Windows.Automation.TreeScope]::Descendants,
        [System.Windows.Automation.AndCondition]::new($named, $typed))
}

function Invoke-Button($button) {
    $button.GetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern).Invoke()
}

function Test-Palette($window, [int]$processId) {
    Add-Type -AssemblyName System.Windows.Forms
    $root = [System.Windows.Automation.AutomationElement]::RootElement
    $element = [System.Windows.Automation.AutomationElement]
    $type = [System.Windows.Automation.ControlType]
    $target = [System.Windows.Automation.AndCondition]::new(
        [System.Windows.Automation.PropertyCondition]::new($element::NameProperty, 'Palette AiWin11'),
        [System.Windows.Automation.PropertyCondition]::new($element::ProcessIdProperty, $processId))
    $palette = {
        $item = $root.FindFirst([System.Windows.Automation.TreeScope]::Children, $target)
        if ($item -and !$item.Current.IsOffscreen) { $item }
    }
    $open = Wait-For { Find-Control $window 'Palette de commandes' $type::Button } 'Palette button is unavailable.'
    $baseline = (Read-TaskEvents 0).last
    $draft = 'Brouillon de validation'
    Invoke-Button $open
    $overlay = Wait-For $palette 'The palette did not open.'
    $field = Wait-For { Find-Control $overlay 'Commande ou brouillon' $type::Edit } 'The palette input did not render.'
    $null = Wait-For { $field.Current.HasKeyboardFocus } 'The palette input did not receive focus.'
    $field.GetCurrentPattern([System.Windows.Automation.ValuePattern]::Pattern).SetValue($draft)
    $prepare = "Pr$([char]0x00e9)parer la demande"
    Invoke-Button (Wait-For { Find-Control $overlay $prepare $type::Button } 'Draft command did not appear.')
    $null = Wait-For {
        $input = Find-Control $window 'Ta demande' $type::Edit
        $input -and $input.GetCurrentPattern([System.Windows.Automation.ValuePattern]::Pattern).Current.Value -eq $draft
    } 'The draft did not reach the main prompt.'

    $shell = New-Object -ComObject WScript.Shell
    if (!$shell.AppActivate($processId)) { throw 'Unable to activate AiWin11 to check keyboard shortcuts.' }
    $prompt = Find-Control $window 'Ta demande' $type::Edit
    $prompt.SetFocus()
    $null = Wait-For { $prompt.Current.HasKeyboardFocus } 'AiWin11 does not have keyboard focus.'
    [System.Windows.Forms.SendKeys]::SendWait('^% ')
    $overlay = Wait-For $palette 'The global shortcut did not open the palette.'
    $null = Wait-For { $element::FocusedElement.Current.Name -eq 'Commande ou brouillon' } 'The shortcut did not focus the palette.'
    [System.Windows.Forms.SendKeys]::SendWait('{ESC}')
    $null = Wait-For { !(& $palette) } 'Escape did not dismiss the palette.'

    for ($i = 0; $i -lt 20; $i++) {
        Invoke-Button $open
        $overlay = Wait-For $palette 'The palette did not reopen.'
        $close = Wait-For { Find-Control $overlay 'Fermer la palette' $type::Button } 'Close button is unavailable.'
        Start-Sleep -Milliseconds 100
        Invoke-Button $close
        $null = Wait-For { !(& $palette) } 'The palette did not close.'
    }
    $events = Read-TaskEvents $baseline
    if ($events.kinds -contains 'task.created') { throw 'The palette started a task while preparing a draft.' }
    if ($events.palette.Count -lt 20) { throw "Expected at least 20 palette samples, got $($events.palette.Count)." }
    $sorted = @($events.palette | Sort-Object)
    $p95 = $sorted[[int][Math]::Ceiling($sorted.Count * 0.95) - 1]
    Write-Output "Palette smoke passed: draft, shortcut, Escape, no task created. p95: ${p95} ms ($($sorted.Count) samples)."
    if ($p95 -ge 200) { throw 'Palette p95 is at or above 200 ms.' }
}

function Test-Displays($window) {
    $buttonType = [System.Windows.Automation.ControlType]::Button
    $inspector = "Inspecteur"
    $null = Wait-For { Find-Control $window 'Accueil' $buttonType } 'The main navigation did not render.'
    $changedMode = !(Find-Control $window $inspector $buttonType)
    $expertToggle = $null
    try {
        if ($changedMode) {
            $expert = Find-Control $window 'Mode expert' $buttonType
            if (!$expert) { throw 'Expert mode is unavailable.' }
            $expertToggle = $expert.GetCurrentPattern([System.Windows.Automation.TogglePattern]::Pattern)
            $expertToggle.Toggle()
        }
        $tab = Wait-For { Find-Control $window $inspector $buttonType } 'Inspector navigation is unavailable.'
        Invoke-Button $tab
        $screens = "$([char]0x00e9)crans"
        $list = Wait-For { Find-Control $window "Lister les $screens" $buttonType } 'Display inspection did not render.'
        $baseline = (Read-TaskEvents 0).last
        Invoke-Button $list
        $null = Wait-For { Find-Control $window "Actualiser les $screens" $buttonType } 'The Windows bridge did not return a display list.'
        $null = Wait-For { (Read-TaskEvents $baseline).brokerDisplay } 'The audit does not show broker execution for the display list.'
        Write-Output 'Desktop smoke passed: display list returned through the Windows broker.'
    } finally {
        if ($expertToggle) {
            $expertToggle.Toggle()
            $null = Wait-For { !(Find-Control $window $inspector $buttonType) } 'Expert mode could not be restored.'
        }
    }
}

$process = Start-Process -FilePath $exe -PassThru
try {
    $window = Wait-For {
        $process.Refresh()
        if ($process.HasExited) { throw "AiWin11 exited with code $($process.ExitCode)." }
        if ($process.MainWindowHandle -ne [IntPtr]::Zero) {
            $candidate = [System.Windows.Automation.AutomationElement]::FromHandle($process.MainWindowHandle)
            if (Find-Control $candidate 'Accueil' ([System.Windows.Automation.ControlType]::Button)) {
                $candidate
            }
        }
    } 'The AiWin11 window did not open.'
    if ($PaletteOnly) {
        Test-Palette $window $process.Id
        return
    }
    if ($ModelsOnly) {
        $baseline = (Read-TaskEvents 0).last
        $models = Find-Control $window "Mod$([char]0x00e8)les" ([System.Windows.Automation.ControlType]::Button)
        if (!$models) { throw 'Models navigation is unavailable.' }
        Invoke-Button $models
        $null = Wait-For { (Read-TaskEvents $baseline).speechCatalog } 'The Windows speech catalog did not return.'
        Write-Output 'Desktop smoke passed: Models page and Windows speech catalog, with no microphone capture.'
        return
    }
    if ($InspectOnly) {
        Test-Displays $window
        return
    }
    $editType = [System.Windows.Automation.ControlType]::Edit
    $buttonType = [System.Windows.Automation.ControlType]::Button
    $edit = Wait-For { Find-Control $window 'Ta demande' $editType } 'Prompt is not exposed to UI Automation.'
    $send = Wait-For { Find-Control $window 'Envoyer' $buttonType } 'Send button not found.'
    $skip = Find-Control $window 'Aller au contenu principal' $buttonType
    if (!$skip) { throw 'The accessible skip control was not found.' }
    Invoke-Button $skip
    $null = Wait-For {
        [System.Windows.Automation.AutomationElement]::FocusedElement.Current.Name -eq 'Espace de travail'
    } 'The skip control did not focus the main heading.'
    $baseline = (Read-TaskEvents 0).last
    $edit.GetCurrentPattern([System.Windows.Automation.ValuePattern]::Pattern).SetValue('Ouvre le Bloc-notes.')
    foreach ($vendor in 'OpenAI', 'DeepSeek', 'Google Gemini', 'Anthropic Claude') {
        $label = "Envoyer le texte de cette demande $([char]0x00e0) $vendor"
        $consent = Find-Control $window $label ([System.Windows.Automation.ControlType]::CheckBox)
        if ($consent) {
            if (!$AllowCloud) { throw 'The selected model is cloud. Pass -AllowCloud for the synthetic request.' }
            $toggle = $consent.GetCurrentPattern([System.Windows.Automation.TogglePattern]::Pattern)
            if ($toggle.Current.ToggleState -eq [System.Windows.Automation.ToggleState]::Off) { $toggle.Toggle() }
            break
        }
    }
    $null = Wait-For { $send.Current.IsEnabled } 'Send remains disabled. Check the configured provider and model.'
    Invoke-Button $send
    $refuse = Wait-For { Find-Control $window 'Refuser' $buttonType } 'No action was proposed for approval.' 45
    $null = Wait-For { $refuse.Current.HasKeyboardFocus } 'The approval did not focus Refuser.'
    Invoke-Button $refuse
    $terminal = Wait-For {
        $events = (Read-TaskEvents $baseline).kinds
        if ($events -contains 'task.failed' -or $events -contains 'task.cancelled') { throw 'The task did not complete.' }
        if ($events -contains 'task.completed') { $events }
    } 'No completed task was recorded.' 45
    if ($terminal -notcontains 'policy.approval.declined' -or $terminal -contains 'tool.execution.completed') {
        throw 'The audit does not prove refusal without execution.'
    }
    Write-Output 'Desktop smoke passed: skip focus, approval focus, refusal and task audit.'
} finally {
    if (!$process.HasExited) {
        $null = $process.CloseMainWindow()
        if (!$process.WaitForExit(5000)) { Stop-Process -InputObject $process -Force }
    }
}
