; Instalador do Sensoryx (Inno Setup 6.1 ou mais novo).
; Gere com installer\build-installer.ps1 (faz o cargo build --release antes).

#define AppName "Sensoryx"
; A versão vem do Cargo.toml (build-installer.ps1 passa /DAppVersion=...); este é só o valor de reserva.
#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif
#define AppExe "sensoryx.exe"
#define Root ".."

[Setup]
AppId={{B7E0C1B2-5C1A-4F43-9E7B-5E4D3A1F2C10}
AppName={#AppName}
AppVersion={#AppVersion}
AppPublisher=PEDROTX SERVICES
LicenseFile=..\LICENSE
DefaultDirName={autopf}\{#AppName}
DefaultGroupName={#AppName}
DisableProgramGroupPage=yes
PrivilegesRequired=admin
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
OutputDir=Output
OutputBaseFilename=SensoryxSetup-{#AppVersion}
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
UninstallDisplayName={#AppName}
UninstallDisplayIcon={app}\{#AppExe}
InfoBeforeFile=info.txt
CloseApplications=no

[Languages]
Name: "brazilianportuguese"; MessagesFile: "compiler:Languages\BrazilianPortuguese.isl"
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "autostart"; Description: "Iniciar o Sensoryx com o Windows"; GroupDescription: "Opções:"

[Files]
Source: "{#Root}\target\release\{#AppExe}"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#Root}\vendor\pawnio-modules\IntelMSR.bin"; DestDir: "{app}\modules"; Flags: ignoreversion
Source: "{#Root}\vendor\pawnio-modules\AMDFamily17.bin"; DestDir: "{app}\modules"; Flags: ignoreversion
; Código-fonte dos módulos LGPL-2.1 distribuídos (versão 0.2.12), para cumprir a licença.
Source: "{#Root}\vendor\pawnio-modules-src\IntelMSR.p"; DestDir: "{app}\licenses\source"; Flags: ignoreversion
Source: "{#Root}\vendor\pawnio-modules-src\AMDFamily17.p"; DestDir: "{app}\licenses\source"; Flags: ignoreversion
Source: "{#Root}\vendor\pawnio-modules\COPYING"; DestDir: "{app}\licenses"; DestName: "COPYING-PawnIO-Modules-LGPL21.txt"; Flags: ignoreversion
Source: "{#Root}\vendor\pawnio-license\COPYING-PawnIO-GPL2.txt"; DestDir: "{app}\licenses"; Flags: ignoreversion
Source: "{#Root}\THIRD_PARTY_NOTICES.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#Root}\LICENSE"; DestDir: "{app}"; DestName: "LICENSE.txt"; Flags: ignoreversion

[Run]
Filename: "schtasks.exe"; Parameters: "/Create /TN ""{#AppName}"" /TR ""\""{app}\{#AppExe}\"""" /SC ONLOGON /RL HIGHEST /F"; Flags: runhidden; Tasks: autostart; StatusMsg: "Configurando início automático..."
Filename: "{app}\{#AppExe}"; Description: "Abrir o {#AppName}"; Flags: nowait postinstall skipifsilent
; Instalação silenciosa (usada pela atualização dentro do app): reabre o Sensoryx ao terminar.
Filename: "{app}\{#AppExe}"; Flags: nowait; Check: WizardSilent

[UninstallRun]
Filename: "taskkill.exe"; Parameters: "/F /IM {#AppExe}"; Flags: runhidden; RunOnceId: "KillApp"
Filename: "schtasks.exe"; Parameters: "/Delete /TN ""{#AppName}"" /F"; Flags: runhidden; RunOnceId: "DelTask"

[Code]
const
  { Versão oficial do instalador do PawnIO (licença GPL-2.0), baixada em vez de redistribuída. }
  PawnUrl = 'https://github.com/namazso/PawnIO.Setup/releases/download/2.2.0/PawnIO_setup.exe';
  PawnSha256 = '1f519a22e47187f70a1379a48ca604981c4fcf694f4e65b734aaa74a9fba3032';
  PawnSite = 'https://pawnio.eu';

function NeedsPawnIO: Boolean;
begin
  Result := not RegKeyExists(HKLM, 'SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\PawnIO');
end;

{ WebView2 Runtime (acompanha o Windows 11): necessário só para a tela de configurações. }
function HasWebView2: Boolean;
var
  V: String;
  K: String;
begin
  K := 'SOFTWARE\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}';
  Result := (RegQueryStringValue(HKLM32, K, 'pv', V) and (V <> '') and (V <> '0.0.0.0'))
         or (RegQueryStringValue(HKLM64, K, 'pv', V) and (V <> '') and (V <> '0.0.0.0'))
         or (RegQueryStringValue(HKCU, K, 'pv', V) and (V <> '') and (V <> '0.0.0.0'));
end;

{ Fecha uma instância em execução (ela roda como administrador e trava o .exe). }
function PrepareToInstall(var NeedsRestart: Boolean): String;
var
  Code: Integer;
begin
  Exec(ExpandConstant('{sys}\taskkill.exe'), '/F /IM {#AppExe}', '', SW_HIDE, ewWaitUntilTerminated, Code);
  Result := '';
end;

procedure InstallPawnIO;
var
  Code: Integer;
begin
  if not NeedsPawnIO then
    Exit;
  try
    WizardForm.StatusLabel.Caption := 'Baixando o driver PawnIO...';
    DownloadTemporaryFile(PawnUrl, 'PawnIO_setup.exe', PawnSha256, nil);
    WizardForm.StatusLabel.Caption := 'Instalando o driver PawnIO...';
    if (not Exec(ExpandConstant('{tmp}\PawnIO_setup.exe'), '-install -silent', '', SW_HIDE, ewWaitUntilTerminated, Code)) or (Code <> 0) then
      MsgBox('A instalação do driver PawnIO não terminou com sucesso (código ' + IntToStr(Code) + ').' + #13#10 +
             'Sem ele, a temperatura da CPU não aparece. Instale manualmente em ' + PawnSite + '.', mbError, MB_OK);
  except
    MsgBox('Não foi possível baixar e verificar o driver PawnIO:' + #13#10 + GetExceptionMessage + #13#10#13#10 +
           'O Sensoryx foi instalado, mas a temperatura da CPU só aparece depois que você instalar o PawnIO manualmente em ' +
           PawnSite + '.', mbError, MB_OK);
  end;
end;

procedure CurStepChanged(CurStep: TSetupStep);
begin
  if CurStep = ssPostInstall then
  begin
    InstallPawnIO;
    if not HasWebView2 then
      MsgBox('O Microsoft Edge WebView2 Runtime não foi encontrado.' + #13#10 +
             'O Sensoryx funciona normalmente, mas a tela de configurações só abre depois que você instalar o WebView2 em:' + #13#10 +
             'https://developer.microsoft.com/microsoft-edge/webview2/', mbInformation, MB_OK);
  end;
end;
