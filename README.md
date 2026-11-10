# Zcomposer

Orquestrador local de agentes de programação para Windows. Studio e Office compartilharão o mesmo estado de execução. Veja o [plano](PLANO.md).

## Protótipo

O workspace Cargo contém o Core em Rust com SQLite, um adaptador do Codex App Server, uma CLI e uma janela desktop Tauri. Execute na raiz do projeto para as duas interfaces lerem o mesmo banco.

```powershell
cargo run -- demo
cargo run -- sessions
cargo run -- events 0
cargo run -- codex "Responda apenas OK, sem usar ferramentas."
cargo run -- answer 2
cargo run -p zcomposer-desktop
cargo test --workspace
```

Os dados locais ficam em `.zcomposer/state.sqlite`, dentro da pasta de execução. `sessions` mostra o cursor do snapshot. `events <cursor>` retorna até 100 eventos posteriores, em ordem.

O comando `codex` e o botão **Iniciar turno** usam o Codex CLI autenticado, disponível no PATH. Cada turno abre uma conversa efêmera em modo somente leitura, com prazo de dois minutos. A tarefa, o estado e a resposta final ficam no banco local. Use o ID impresso pela CLI ao consultar `answer`.

Studio e Office mostram o mesmo conjunto de sessões e preservam a seleção quando você alterna de vista. Nesta etapa, são uma lista e uma grade de agentes em HTML, CSS e JavaScript. O canvas interativo e o escritório ilustrado entram nas próximas etapas.

No Windows com Rust MSVC, use o Developer PowerShell do Visual Studio caso o linker e o compilador C do SQLite não sejam encontrados.
