# Avisos de terceiros

O Sensoryx usa os componentes abaixo. Eles mantêm as suas próprias licenças.

## PawnIO (driver de kernel) — GPL-2.0

- Projeto: https://github.com/namazso/PawnIO — autor: namazso
- O Sensoryx **não redistribui** o driver: o instalador baixa o `PawnIO_setup.exe` oficial (v2.2.0) do
  release do projeto, confere o hash SHA-256 e o executa só se o driver ainda não estiver instalado.
- O Sensoryx não contém código do driver: ele só se comunica com o driver instalado por IOCTL
  (`\\?\GLOBALROOT\Device\PawnIO`). O arquivo de licença do PawnIO concede uma exceção expressa para
  programas e módulos "que se comunicam com o PawnIO exclusivamente pela interface de controle de E/S
  do dispositivo", que é o caso do Sensoryx; ver o aviso de licença em
  https://github.com/namazso/PawnIO (README, seção License).
- Código-fonte do driver: https://github.com/namazso/PawnIO
- Código-fonte do instalador: https://github.com/namazso/PawnIO.Setup
- Texto da licença: `licenses/COPYING-PawnIO-GPL2.txt`
- O driver **não** é removido na desinstalação do Sensoryx, porque outros programas podem usá-lo.

## PawnIO.Modules (módulos do driver) — LGPL-2.1-or-later

- Projeto: https://github.com/namazso/PawnIO.Modules (versão 0.2.12)
- Arquivos distribuídos sem modificações, na pasta `modules\`: `IntelMSR.bin`, `AMDFamily17.bin`.
- Código-fonte dos módulos distribuídos (versão 0.2.12), incluído na instalação em `licenses\source\`
  (`IntelMSR.p`, `AMDFamily17.p`) e em https://github.com/namazso/PawnIO.Modules/tree/0.2.12
- Texto da licença: `licenses/COPYING-PawnIO-Modules-LGPL21.txt`

## Bibliotecas Rust (compiladas no executável)

| Biblioteca | Licença |
| --- | --- |
| `windows` (Microsoft) | MIT ou Apache-2.0 |
| `libloading` | ISC |
| `wry`, `tao` (Tauri) e dependências | MIT ou Apache-2.0 |
| `sha2` (RustCrypto) | MIT ou Apache-2.0 |

A lista completa de dependências transitivas está em `Cargo.lock`.

## Microsoft Edge WebView2 Runtime

A tela de configurações usa o WebView2 Runtime, componente do sistema que acompanha o Windows 11.
Ele não é distribuído com o Sensoryx.

## NVIDIA NVML

O Sensoryx carrega a `nvml.dll` que acompanha o driver gráfico NVIDIA já instalado no PC do usuário.
Nenhum arquivo da NVIDIA é distribuído com o Sensoryx.
