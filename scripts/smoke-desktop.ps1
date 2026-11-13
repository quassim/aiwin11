<# Runs a release smoke test through Windows UI Automation. -AllowCloud permits
   sending the synthetic prompt to the provider already selected in AiWin11. #>
param([switch]$AllowCloud, [switch]$InspectOnly, [switch]$ModelsOnly, [switch]$PaletteOnly, [string]$Executable,
      [string]$DatabasePath, [switch]$ApproveNotepad, [switch]$StopAtApproval, [switch]$GlobalStop, [switch]$DesktopControls,
      [switch]$MemoryIndex, [switch]$SessionMemory, [switch]$WorkflowCondition, [switch]$FolderTrigger, [switch]$McpProbe, [ValidateSet('Approve','Decline')][string]$PowerShellDiagnostic)

$ErrorActionPreference = 'Stop'
if ($ApproveNotepad -and $StopAtApproval) { throw 'Choose one approval scenario.' }
$exe = if ($Executable) { (Resolve-Path -LiteralPath $Executable).ProviderPath } else {
    Join-Path (Split-Path -Parent $PSScriptRoot) 'target\release\aiwin11-desktop.exe'
}
if (!(Test-Path -LiteralPath $exe -PathType Leaf)) { throw 'Build the release application first.' }
$appNames = @('aiwin11-desktop', 'AiWin11', [IO.Path]::GetFileNameWithoutExtension($exe)) | Select-Object -Unique
if (Get-Process -Name $appNames -ErrorAction SilentlyContinue) { throw 'Close AiWin11 before testing.' }
Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes
$db = if ($DatabasePath) { $DatabasePath } else {
    Join-Path $env:LOCALAPPDATA 'app.aiwin11.desktop\aiwin11.sqlite3'
}
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

function Set-Field($field, [string]$value) {
    $field.GetCurrentPattern([System.Windows.Automation.ValuePattern]::Pattern).SetValue($value)
}

function Test-SessionMemory($window) {
    if ((Split-Path (Split-Path $db -Parent) -Leaf) -ne 'app.aiwin11.recette') {
        throw 'Conversation fixture requires the isolated recipe profile.'
    }
    $type = [System.Windows.Automation.ControlType]
    $edit = Find-Control $window 'Ta demande' $type::Edit
    $send = Find-Control $window 'Envoyer' $type::Button
    $marker = 'AZUR' + [guid]::NewGuid().ToString('N').Substring(0,8).ToUpperInvariant()
    $recall = "Quel est mon code de recette ? R$([char]0xe9)ponds uniquement avec le code, ou INCONNU si tu ne le connais pas."
    $ask = {
        param($prompt, $expected)
        $null = Wait-For { $edit.Current.IsEnabled } 'Prompt is not ready.'
        Set-Field $edit $prompt
        $baseline = (Read-TaskEvents 0).last
        $null = Wait-For { $send.Current.IsEnabled } 'Request is not ready to send.'
        Invoke-Button $send
        $null = Wait-For { (Read-TaskEvents $baseline).kinds -contains 'task.completed' } 'Conversation task did not complete.' 45
        $null = Wait-For {
            $items = $window.FindAll([System.Windows.Automation.TreeScope]::Descendants, [System.Windows.Automation.Condition]::TrueCondition)
            $items | Where-Object { $_.Current.Name.Trim().TrimEnd('.') -ceq $expected } | Select-Object -First 1
        } "Conversation reply did not match $expected."
    }
    & $ask "Mon code de recette est $marker. R$([char]0xe9)ponds uniquement COMPRIS." 'COMPRIS'
    & $ask $recall $marker
    $reset = Wait-For {
        $items = $window.FindAll([System.Windows.Automation.TreeScope]::Descendants, [System.Windows.Automation.Condition]::TrueCondition)
        $items | Where-Object { $_.Current.ControlType -eq $type::Button -and $_.Current.Name -like 'Nouvelle conversation*' -and $_.Current.IsEnabled } | Select-Object -First 1
    } 'New conversation button unavailable.'
    Invoke-Button $reset
    $null = Wait-For {
        $items = $window.FindAll([System.Windows.Automation.TreeScope]::Descendants, [System.Windows.Automation.Condition]::TrueCondition)
        !($items | Where-Object { $_.Current.Name -like 'Nouvelle conversation*' } | Select-Object -First 1)
    } 'Conversation context was not reset.'
    & $ask $recall 'INCONNU'
    Write-Output 'Desktop smoke passed: follow-up recalls a synthetic code and new conversation forgets it.'
}

function Test-FolderTrigger($window) {
    if ((Split-Path (Split-Path $db -Parent) -Leaf) -ne 'app.aiwin11.recette') {
        throw 'Folder trigger requires the isolated recipe profile.'
    }
    $type = [System.Windows.Automation.ControlType]
    $marker = 'RECETTE' + [guid]::NewGuid().ToString('N').Substring(0,8)
    $prompt = "R$([char]0xe9)ponds uniquement $marker."
    $fixture = Join-Path (Split-Path $db -Parent) "files/$marker.txt"
    Set-Field (Find-Control $window 'Ta demande' $type::Edit) $prompt
    $saveLabel = 'Enregistrer la demande'
    Invoke-Button (Wait-For { Find-Control $window $saveLabel $type::Button } 'Save request unavailable.')
    $entry = Wait-For { Find-Control $window $prompt $type::Button } 'Saved request did not appear.'
    $walker = [System.Windows.Automation.TreeWalker]::ControlViewWalker
    $row = $walker.GetParent($entry)
    try {
        Invoke-Button (Wait-For { Find-Control $row 'Quand le dossier change' $type::Button } 'Folder trigger unavailable.')
        $null = Wait-For { Find-Control $row "D$([char]0xe9)planifier" $type::Button } 'Folder trigger was not saved.'
        $baseline = (Read-TaskEvents 0).last
        [IO.File]::WriteAllText($fixture, 'fixture')
        $null = Wait-For {
            $events = (Read-TaskEvents $baseline).kinds
            $events -contains 'task.completed'
        } 'Folder change did not complete a scheduled task.' 75
        Invoke-Button (Find-Control $window "Centre de contr$([char]0xf4)le" $type::Button)
        Invoke-Button (Find-Control $window 'Accueil' $type::Button)
        $null = Wait-For {
            $items = $window.FindAll([System.Windows.Automation.TreeScope]::Descendants, [System.Windows.Automation.Condition]::TrueCondition)
            $items | Where-Object { $_.Current.Name -match "$marker.*$marker" } | Select-Object -First 1
        } 'Scheduled report was not rendered.'
        Write-Output 'Desktop smoke passed: folder change started a local task and produced a visible report.'
    } finally {
        $entry = Find-Control $window $prompt $type::Button
        if ($entry) {
            $row = $walker.GetParent($entry)
            $remove = Find-Control $row 'Supprimer cette demande' $type::Button
            if ($remove) { Invoke-Button $remove }
            $null = Wait-For { !(Find-Control $window $prompt $type::Button) } 'Saved fixture was not removed.'
        }
        Remove-Item -LiteralPath $fixture -ErrorAction SilentlyContinue
    }
}

function Test-McpProbe($window) {
    if ((Split-Path (Split-Path $db -Parent) -Leaf) -ne 'app.aiwin11.recette') {
        throw 'MCP fixture requires the isolated recipe profile.'
    }
    $type = [System.Windows.Automation.ControlType]
    $fixture = Join-Path (Split-Path $db -Parent) ('mcp-' + [guid]::NewGuid().ToString('N') + '.py')
    @'
import json, sys
for line in sys.stdin:
    request = json.loads(line)
    method = request.get('method')
    if 'id' not in request:
        continue
    if method == 'server/discover':
        result = {'resultType': 'complete', 'supportedVersions': ['2026-07-28'], 'capabilities': {'tools': {}},
                  '_meta': {'io.modelcontextprotocol/serverInfo': {'name': 'AiWin11 fixture', 'version': '1'}}}
    elif method == 'tools/list':
        result = {'resultType': 'complete', 'tools': [{'name': 'read_fixture', 'inputSchema': {'type': 'object'}}]}
    else:
        result = {'resultType': 'complete', 'content': [{'type': 'text', 'text': 'AZURQUINZE'}]}
    print(json.dumps({'jsonrpc': '2.0', 'id': request['id'], 'result': result}), flush=True)
'@ | Set-Content -LiteralPath $fixture -Encoding ascii
    try {
        $extensions = Find-Control $window 'Extensions' $type::Button
        if (!$extensions) {
            $expert = Find-Control $window 'Mode expert' $type::Button
            $expert.GetCurrentPattern([System.Windows.Automation.TogglePattern]::Pattern).Toggle()
            $extensions = Wait-For { Find-Control $window 'Extensions' $type::Button } 'Extensions navigation unavailable.'
        }
        Invoke-Button $extensions
        Invoke-Button (Wait-For { Find-Control $window 'Ajouter une extension' $type::Button } 'Add extension unavailable.')
        Set-Field (Find-Control $window 'Nom' $type::Edit) 'recette_mcp'
        Set-Field (Find-Control $window "Ex$([char]0xe9)cutable (chemin complet)" $type::Edit) $python
        Set-Field (Find-Control $window 'Arguments (un par ligne)' $type::Edit) "-I`n-B`n$fixture"
        Set-Field (Find-Control $window "Outils autoris$([char]0xe9)s (un identifiant par ligne)" $type::Edit) 'read_fixture'
        $enabled = Find-Control $window 'Activer ce serveur' $type::CheckBox
        $enabled.GetCurrentPattern([System.Windows.Automation.TogglePattern]::Pattern).Toggle()
        Invoke-Button (Find-Control $window 'Enregistrer' $type::Button)
        $test = Wait-For { $candidate = Find-Control $window 'Tester' $type::Button; if ($candidate -and $candidate.Current.IsEnabled) { $candidate } } 'MCP fixture was not saved.'
        Invoke-Button $test
        $null = Wait-For {
            $items = $window.FindAll([System.Windows.Automation.TreeScope]::Descendants, [System.Windows.Automation.Condition]::TrueCondition)
            $items | Where-Object { $_.Current.Name -match '2026-07-28.*read_fixture' } | Select-Object -First 1
        } 'MCP version and tool were not shown.'
        Add-Content -LiteralPath $fixture -Encoding ascii -Value '# fixture update'
        Invoke-Button $test
        $accept = Wait-For { Find-Control $window 'Accepter la nouvelle version' $type::Button } 'Changed MCP fixture was not flagged.'
        Invoke-Button $accept
        $null = Wait-For { !(Find-Control $window 'Accepter la nouvelle version' $type::Button) } 'MCP fixture update was not accepted.'
        Invoke-Button (Find-Control $window 'Tester' $type::Button)
        $null = Wait-For {
            $items = $window.FindAll([System.Windows.Automation.TreeScope]::Descendants, [System.Windows.Automation.Condition]::TrueCondition)
            $items | Where-Object { $_.Current.Name -match '2026-07-28.*read_fixture' } | Select-Object -First 1
        } 'MCP fixture did not respond after accepting the update.'
        Invoke-Button (Find-Control $window 'Accueil' $type::Button)
        $prompt = Wait-For { Find-Control $window 'Ta demande' $type::Edit } 'Prompt unavailable.'
        $send = Find-Control $window 'Envoyer' $type::Button
        foreach ($case in @(
            @{ Prompt = 'Utilise mcp_list_tools avec le serveur recette_mcp, puis donne la liste.'; Title = "L$([char]0x2019)agent demande $([char]0xe0) lister les outils d$([char]0x2019)une extension" },
            @{ Prompt = 'Appelle directement mcp_call pour read_fixture sur recette_mcp avec arguments {}, puis donne son texte.'; Title = "L$([char]0x2019)agent propose d$([char]0x2019)utiliser une extension" }
        )) {
            $null = Wait-For { $prompt.Current.IsEnabled } 'Previous MCP task still holds the prompt.'
            $baseline = (Read-TaskEvents 0).last
            Set-Field $prompt $case.Prompt
            $null = Wait-For { $send.Current.IsEnabled } 'MCP request is not ready to send.'
            Invoke-Button $send
            $null = Wait-For { Find-Control $window $case.Title $type::Text } 'Expected MCP approval did not appear.' 40
            Invoke-Button (Find-Control $window 'Autoriser' $type::Button)
            $null = Wait-For {
                $events = (Read-TaskEvents $baseline).kinds
                ($events -contains 'task.completed') -and ($events -contains 'mcp.request.completed')
            } 'MCP request completion missing from the audit.' 45
        }
        Write-Output 'Desktop smoke passed: MCP fixture probe and update acceptance.'
    } finally {
        $extensions = Find-Control $window 'Extensions' $type::Button
        if ($extensions) { Invoke-Button $extensions }
        $null = Wait-For { Find-Control $window 'Ajouter une extension' $type::Button } 'Extensions did not reopen.'
        $remove = Find-Control $window 'Retirer' $type::Button
        if ($remove) {
            Invoke-Button $remove
            $null = Wait-For { !(Find-Control $window 'Retirer' $type::Button) } 'MCP fixture removal was not rendered.'
            Invoke-Button (Find-Control $window 'Enregistrer' $type::Button)
            Start-Sleep -Milliseconds 300
        }
        Remove-Item -LiteralPath $fixture -ErrorAction SilentlyContinue
    }
}

function Test-MemoryIndex($window) {
    if ((Split-Path (Split-Path $db -Parent) -Leaf) -ne 'app.aiwin11.recette') {
        throw 'Memory fixture requires the isolated recipe profile.'
    }
    $type = [System.Windows.Automation.ControlType]
    $file = 'recette-' + [guid]::NewGuid().ToString('N') + '.txt'
    $fixture = Join-Path (Split-Path $db -Parent) "files/$file"
    [IO.File]::WriteAllText($fixture, 'Le code de recette est azurquinze.')
    try {
        Invoke-Button (Wait-For { Find-Control $window "M$([char]0x00e9)moire" $type::Button } 'Memory navigation unavailable.')
        Set-Field (Wait-For { Find-Control $window 'Nom du fichier (.txt, .md ou .csv)' $type::Edit } 'Index input unavailable.') $file
        Invoke-Button (Wait-For { Find-Control $window 'Indexer le fichier' $type::Button } 'Index button unavailable.')
        $removeLabel = "Effacer l$([char]0x2019)extrait"
        $remove = Wait-For { Find-Control $window $removeLabel $type::Button } 'Indexed file did not appear.'
        Set-Field (Wait-For { Find-Control $window 'Rechercher dans les extraits' $type::Edit } 'Search field unavailable.') 'azurquinze'
        Invoke-Button (Wait-For { Find-Control $window 'Rechercher' $type::Button } 'Search button unavailable.')
        $null = Wait-For { Find-Control $window $file $type::Button } 'Search result did not appear.'
        Invoke-Button $remove
        $null = Wait-For { !(Find-Control $window $removeLabel $type::Button) } 'Index entry was not removed.'
        Write-Output 'Desktop smoke passed: indexed a fixture, found it by its content and removed the entry.'
    } finally {
        Remove-Item -LiteralPath $fixture -ErrorAction SilentlyContinue
    }
}

function Test-DesktopControls($window) {
    $type = [System.Windows.Automation.ControlType]
    $expert = Wait-For { Find-Control $window 'Mode expert' $type::Button } 'Expert switch unavailable.'
    $toggle = $expert.GetCurrentPattern([System.Windows.Automation.TogglePattern]::Pattern)
    $initial = $toggle.Current.ToggleState
    try {
        $toggle.Toggle()
        $null = Wait-For { $toggle.Current.ToggleState -ne $initial } 'Mode switch did not change state.'
        $null = Wait-For {
            $visible = $null -ne (Find-Control $window 'Inspecteur' $type::Button)
            $visible -eq ($toggle.Current.ToggleState -eq [System.Windows.Automation.ToggleState]::On)
        } 'Inspector visibility did not follow the expert switch.'
    } finally {
        if ($toggle.Current.ToggleState -ne $initial) { $toggle.Toggle() }
    }
    $open = Wait-For { Find-Control $window 'Mode flottant' $type::Button } 'Floating control unavailable.'
    $before = $window.Current.BoundingRectangle.Width
    $open.GetCurrentPattern([System.Windows.Automation.TogglePattern]::Pattern).Toggle()
    $close = Wait-For { Find-Control $window 'Quitter le mode flottant' $type::Button } 'Floating mode did not open.'
    $null = Wait-For { $window.Current.BoundingRectangle.Width -lt $before } 'Floating mode did not shrink the window.'
    $close.GetCurrentPattern([System.Windows.Automation.TogglePattern]::Pattern).Toggle()
    $null = Wait-For { $window.Current.BoundingRectangle.Width -ge $before } 'Normal window size was not restored.'
    $dashboard = "Centre de contr$([char]0x00f4)le"
    Invoke-Button (Wait-For { Find-Control $window $dashboard $type::Button } 'Dashboard navigation unavailable.')
    $null = Wait-For {
        Find-Control $window 'Consommation cloud' $type::Text
    } 'Dashboard did not render the cloud usage section.'
    $refresh = Wait-For { Find-Control $window 'Actualiser' $type::Button } 'Dashboard refresh unavailable.'
    $null = Wait-For { $refresh.Current.IsEnabled } 'Dashboard did not finish loading.'
    Invoke-Button $refresh
    $null = Wait-For { $refresh.Current.IsEnabled } 'Dashboard did not finish refreshing.'
    Write-Output 'Desktop smoke passed: simple/expert mode, floating window and dashboard.'
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
        $null = Wait-For { Find-Control $window "Actualiser les $screens" $buttonType } 'The Windows bridge did not return a display list.' 27
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
    if ($SessionMemory) {
        Test-SessionMemory $window
        return
    }
    if ($FolderTrigger) {
        Test-FolderTrigger $window
        return
    }
    if ($McpProbe) {
        Test-McpProbe $window
        return
    }
    if ($MemoryIndex) {
        Test-MemoryIndex $window
        return
    }
    if ($DesktopControls) {
        Test-DesktopControls $window
        return
    }
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
    if ($WorkflowCondition) {
        if ($AllowCloud) { throw 'Workflow fixture requires a local provider.' }
        $header = "Ex$([char]0xe9)cute ces $([char]0xe9)tapes dans l$([char]0x2019)ordre. Si une $([char]0xe9)tape $([char]0xe9)choue, arr$([char]0xea)te-toi et explique pourquoi."
        Set-Field $edit "$header`n1. [r$([char]0xe9)essayer] R$([char]0xe9)ponds uniquement AZURQUINZE.`n2. [si contient:AZURQUINZE] R$([char]0xe9)ponds uniquement VERTSEIZE.`n3. [si contient:IMPOSSIBLEXYZ] R$([char]0xe9)ponds uniquement ROUGEVINGT."
    } elseif ($PowerShellDiagnostic) {
        if ($AllowCloud) { throw 'Diagnostic fixture requires a local provider.' }
        Set-Field $edit "Utilise run_powershell_diagnostic pour lire l$([char]0x2019)$([char]0xe9)dition et le num$([char]0xe9)ro de build de Windows."
    } else {
        Set-Field $edit 'Ouvre le Bloc-notes.'
    }
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
    if ($WorkflowCondition) {
        $null = Wait-For {
            $events = (Read-TaskEvents $baseline).kinds
            if ($events -contains 'task.failed') { throw 'Workflow failed.' }
            ($events -contains 'task.completed') -and
                (@($events | Where-Object { $_ -eq 'workflow.step.completed' }).Count -eq 2) -and
                ($events -contains 'workflow.step.condition.unmet')
        } 'Workflow condition results missing from audit.' 60
        Write-Output 'Desktop smoke passed: two workflow steps completed and one condition skipped.'
        return
    }
    $refuse = Wait-For { Find-Control $window 'Refuser' $buttonType } 'No action was proposed for approval.' 45
    $null = Wait-For { $refuse.Current.HasKeyboardFocus } 'The approval did not focus Refuser.'
    if ($PowerShellDiagnostic) {
        $null = Wait-For {
            Find-Control $window 'Autoriser le diagnostic PowerShell' ([System.Windows.Automation.ControlType]::Text)
        } 'The action proposed was not the fixed PowerShell diagnostic.'
        $decision = if ($PowerShellDiagnostic -eq 'Approve') {
            Wait-For { Find-Control $window 'Autoriser' $buttonType } 'Approve button unavailable.'
        } else { $refuse }
        Invoke-Button $decision
        $null = Wait-For {
            $events = (Read-TaskEvents $baseline).kinds
            if ($events -contains 'task.failed') { throw 'Diagnostic task failed.' }
            if ($events -contains 'task.completed') {
                if ($PowerShellDiagnostic -eq 'Approve') {
                    if ($events -notcontains 'diagnostic.run.completed' -or $events -notcontains 'policy.approval.granted') {
                        throw 'Approved diagnostic missing from audit.'
                    }
                } elseif ($events -notcontains 'policy.approval.declined' -or $events -contains 'diagnostic.run.completed') {
                    throw 'Diagnostic refusal was not enforced.'
                }
                $true
            }
        } 'Diagnostic task did not finish.' 50
        Write-Output "Desktop smoke passed: PowerShell diagnostic $PowerShellDiagnostic."
        return
    }
    if ($StopAtApproval -or $GlobalStop) {
        $stopLabel = "Stoppe toutes les t$([char]0x00e2)ches"
        if ($GlobalStop) {
            Add-Type -AssemblyName System.Windows.Forms
            if (!$refuse.Current.HasKeyboardFocus) { throw 'AiWin11 does not have keyboard focus.' }
            [System.Windows.Forms.SendKeys]::SendWait('^%{BACKSPACE}')
        } else {
            Invoke-Button (Wait-For { Find-Control $window $stopLabel $buttonType } 'Stop button is unavailable.')
        }
        $null = Wait-For {
            $events = (Read-TaskEvents $baseline).kinds
            if ($events -contains 'tool.execution.completed') { throw 'An action ran after stop.' }
            $events -contains 'task.cancelled'
        } 'The pending task was not cancelled.'
        $trigger = if ($GlobalStop) { 'global shortcut' } else { 'stop button' }
        Write-Output "Desktop smoke passed: $trigger cancels a pending approval without execution."
        return
    }
    if ($ApproveNotepad) {
        if (Get-Process notepad -ErrorAction SilentlyContinue) { throw 'Close Notepad before the launch test.' }
        $approved = Wait-For { Find-Control $window 'Autoriser' $buttonType } 'Approve button unavailable.'
        Invoke-Button $approved
        $null = Wait-For {
            $events = (Read-TaskEvents $baseline).kinds
            ($events -contains 'policy.approval.granted') -and ($events -contains 'tool.execution.completed')
        } 'Approved Notepad launch missing from the audit.' 35
        $null = Wait-For { Get-Process notepad -ErrorAction SilentlyContinue } 'Notepad was not launched.'
        Write-Output 'Desktop smoke passed: approved Notepad launch and broker audit.'
        return
    }
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
