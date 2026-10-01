# Захват экрана в PNG для скриншотов документации.
# Запуск:  powershell -ExecutionPolicy Bypass -File D:\coding\odata4-public\docs\_raw\capture.ps1
# Скрипт ждёт файл _raw\shot.txt (в нём имя снимка), делает снимок всего экрана в _raw\<имя>.png
# и удаляет shot.txt. Остановить — Ctrl+C или закрыть окно.

Add-Type -AssemblyName System.Windows.Forms, System.Drawing
Add-Type 'using System.Runtime.InteropServices; public class DpiFix { [DllImport("user32.dll")] public static extern bool SetProcessDPIAware(); }'
[DpiFix]::SetProcessDPIAware() | Out-Null

$dir = Split-Path -Parent $MyInvocation.MyCommand.Path
$trigger = Join-Path $dir 'shot.txt'
Write-Host "Жду команды на снимок в $trigger ..."

while ($true) {
    if (Test-Path $trigger) {
        Start-Sleep -Milliseconds 400
        $name = (Get-Content $trigger -Raw).Trim()
        Remove-Item $trigger -Force
        if (-not $name) { $name = 'shot-' + (Get-Date -Format 'HHmmss') }
        $b = [System.Windows.Forms.Screen]::PrimaryScreen.Bounds
        $bmp = New-Object System.Drawing.Bitmap $b.Width, $b.Height
        $g = [System.Drawing.Graphics]::FromImage($bmp)
        $g.CopyFromScreen($b.Location, [System.Drawing.Point]::Empty, $b.Size)
        $out = Join-Path $dir ($name + '.png')
        $bmp.Save($out, [System.Drawing.Imaging.ImageFormat]::Png)
        $g.Dispose(); $bmp.Dispose()
        Write-Host "Снимок: $out"
    }
    Start-Sleep -Milliseconds 250
}
