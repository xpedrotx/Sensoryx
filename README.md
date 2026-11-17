# Sensoryx

Monitor leve para a barra de tarefas do Windows 11, escrito em Rust: temperaturas de CPU e GPU,
FPS do app em primeiro plano e bateria de periféricos Logitech.

> Status: versão inicial (0.1.0). Veja "Limitações".

## O que mostra

| Item | Como é lido |
| --- | --- |
| Temperatura da CPU (Intel e AMD Ryzen) | MSR/SMN pelo driver [PawnIO](https://github.com/namazso/PawnIO) |
| Temperatura da GPU NVIDIA | `nvml.dll` do driver gráfico NVIDIA |
| FPS | eventos ETW de apresentação de quadros (DXGI e D3D9) do processo em primeiro plano |
| Bateria Logitech | HID++ 2.0 (em desenvolvimento; ainda não validada com hardware ligado) |

## Requisitos

- Windows 11 (64 bits). O app roda como administrador (necessário para o driver e para o ETW).
- Driver PawnIO, que o instalador baixa e instala se não existir.

## Limitações conhecidas

- FPS: jogos Vulkan/OpenGL que não passam pelo DXGI não geram eventos.
- Temperatura de CPU AMD: implementada a partir do código-fonte do módulo, ainda **não testada** em um Ryzen.
- Bateria Logitech: ainda não validada; headsets e receptores desligados não respondem.
- Só a barra de tarefas horizontal do monitor principal é tratada.

## Privacidade

Este programa não envia nenhuma informação para outros sistemas em rede, a menos que o usuário peça
explicitamente. O único acesso à internet é o do **instalador**, que baixa o instalador oficial do
PawnIO (conferindo o SHA-256).

## Compilar

```powershell
cargo build --release
powershell -ExecutionPolicy Bypass -File installer\build-installer.ps1   # requer Inno Setup 6
```

O instalador precisa da pasta `vendor/` (não versionada) com os módulos do PawnIO; veja o cabeçalho de
`installer/build-installer.ps1`.

## Código-fonte e atualizações

- Código-fonte e versões: <https://github.com/xpedrotx/Sensoryx>
- A aba **Atualizações** da tela de configurações consulta a última versão publicada nesse repositório
  e só baixa o instalador se o hash SHA-256 (arquivo `SHA256SUMS.txt` do release) conferir.

## Licença

[MIT](LICENSE) © 2026 PEDROTX SERVICES. Componentes de terceiros: veja [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).
