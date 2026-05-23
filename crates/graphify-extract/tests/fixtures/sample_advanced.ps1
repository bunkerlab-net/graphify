using namespace System.Collections.Generic
using namespace System.Net.Http
using module MyHelper

# DSC config
Configuration MyWebServer {
    Node "localhost" {
        WindowsFeature Web {
            Name = "Web-Server"
            Ensure = "Present"
        }
    }
}

# Enum
enum Severity {
    Low = 0
    Medium = 1
    High = 2
    Critical = 3
}

# Class with inheritance and static methods
class Logger {
    [string]$Name
    static [int]$Count = 0

    Logger([string]$name) {
        $this.Name = $name
        [Logger]::Count++
    }

    [void] Write([Severity]$level, [string]$msg) {
        Write-Output "[$($this.Name)] [$level] $msg"
    }

    static [Logger] Default() {
        return [Logger]::new("default")
    }
}

class AuditLogger : Logger {
    [string]$Audit

    AuditLogger([string]$name, [string]$audit) : base($name) {
        $this.Audit = $audit
    }

    [void] Audit([string]$msg) {
        $this.Write([Severity]::High, $msg)
    }
}

# Function with advanced parameters
function Invoke-Task {
    [CmdletBinding(SupportsShouldProcess)]
    param(
        [Parameter(Mandatory)]
        [string]$Name,
        [Parameter(ValueFromPipeline)]
        [object[]]$Items,
        [switch]$Force
    )
    begin {
        $logger = [Logger]::Default()
    }
    process {
        foreach ($item in $Items) {
            if ($PSCmdlet.ShouldProcess($item, $Name)) {
                $logger.Write([Severity]::Medium, "processing $item")
            }
        }
    }
    end {
        $logger.Write([Severity]::Low, "done")
    }
}

# Workflow (deprecated but still parsed)
function Test-Pipeline {
    1..3 | Invoke-Task -Name "test"
}

Export-ModuleMember -Function Invoke-Task, Test-Pipeline
