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

Na janela desktop, **Interromper** encerra o turno ativo e registra a interrupção no histórico. Você pode iniciar outra tarefa quando o turno anterior terminar de encerrar.
Se você fechar a janela durante um turno, ela aguarda o encerramento e a gravação do estado antes de sair.

Studio e Office mostram o mesmo conjunto de sessões e preservam a seleção quando você alterna de vista. No Studio, arraste os cartões entre posições da grade ou use os botões de seta. O projeto salva a ordem no SQLite e a restaura ao reabrir. No Office, cada sessão tem uma estação de trabalho que reflete seu estado. Abra um cartão ou uma estação para consultar a tarefa, o histórico de estados e a resposta quando disponível.

Selecione um turno ativo iniciado na janela para acompanhar a resposta enquanto o Codex escreve. O texto parcial fica na memória. Quando o turno conclui, a resposta final fica no SQLite.

Selecione **Revisar commit** na janela ou use `cargo run -- review HEAD` para enviar o diff de um commit ao Codex e registrar o parecer. Você pode indicar outro commit no lugar de `HEAD`. A revisão aceita diffs de até 64 KiB e aparece como uma sessão `codex-review` no Studio e no Office.

Depois de ler o parecer, use **Aprovar revisão** ou **Solicitar ajustes**. A decisão fica salva na sessão. Para registrar uma nova decisão, inicie outra revisão. Esses botões registram o resultado da análise no Zcomposer.

No Windows com Rust MSVC, use o Developer PowerShell do Visual Studio caso o linker e o compilador C do SQLite não sejam encontrados.
