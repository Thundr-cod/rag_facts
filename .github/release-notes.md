Unsigned builds of the facts. launcher. Copy each file into the drive's `.system/` folder, next to the models. See the README for the AI engine and model downloads.

| File | For |
|---|---|
| `facts-windows.exe` | Windows 10/11 x64 |
| `facts-macos` | macOS, Apple Silicon and Intel (one universal file) |
| `facts-linux` | Linux x64, any distro |

**First launch on an unsigned build**

- **Windows:** `WindowsLaunch.bat` clears the downloaded-file mark itself. If SmartScreen still warns, choose **More info → Run anyway**. If the window closes at once, check Windows Security → Protection history, restore the file and allow it.
- **macOS:** right-click `MacLaunch.command` → **Open** → **Open** (on macOS 15: System Settings → Privacy & Security → **Open Anyway**). Needed once; the launcher clears quarantine on everything else.
- **Linux:** nothing extra; the launcher sets the executable bit.

**Verify your download** against `SHA256SUMS.txt`:

- Windows (PowerShell): `Get-FileHash facts-windows.exe`
- macOS: `shasum -a 256 facts-macos`
- Linux: `sha256sum -c SHA256SUMS.txt --ignore-missing`
