# Porteiro

![Ilustração em pixel art do Porteiro, um navegador de hosts SSH](image.png)

[![CI](https://github.com/jujubetsz/porteiro/actions/workflows/ci.yml/badge.svg)](https://github.com/jujubetsz/porteiro/actions/workflows/ci.yml)
[![Licença: Apache-2.0](https://img.shields.io/badge/licen%C3%A7a-Apache--2.0-blue.svg)](LICENSE)

Porteiro é uma interface de terminal (TUI), voltada ao uso pelo teclado, para
navegar pelos hosts definidos na configuração do cliente OpenSSH e conectar
usando o `ssh` já instalado na sua máquina.

A configuração SSH é a única fonte de verdade. O Porteiro não armazena
credenciais, não implementa o protocolo SSH, não testa a disponibilidade dos
hosts e não modifica arquivos de configuração.

Exemplo da interface, que permanece em inglês:

```text
 porteiro  OpenSSH config hosts                                                               v0.2.0
╭ Hosts - 2 shown / 2 total ─────────────────────────────────╮╭ Details ───────────────────────────╮
│  Alias                  HostName                     Port  ││Alias        production             │
│› production             prod.example.test            2222  ││HostName     prod.example.test      │
│  staging                staging.example.test         22    ││User         deploy                 │
│                                                            ││Port         2222                   │
│                                                            ││ProxyJump    bastion                │
│                                                            ││ProxyCommand -                      │
│                                                            ││IdentityFile ~/.ssh/id_rsa          │
│                                                            ││             ~/.ssh/id_ecdsa        │
│                                                            ││             ~/.ssh/id_ecdsa_sk     │
│                                                            ││             ~/.ssh/id_ed25519      │
│                                                            ││             ~/.ssh/id_ed25519_sk   │
╰────────────────────────────────────────────────────────────╯╰────────────────────────────────────╯
 2 host(s) | ~/.ssh/config
 j/k move  / search  Enter connect  i details  r reload  v verbose  ? help  q quit
```

## Funcionalidades

- Lê `~/.ssh/config` por padrão e segue as diretivas `Include` na ordem de
  declaração, inclusive quando os caminhos dependem do destino.
- Lista aliases concretos de `Host` que podem ser passados literalmente ao
  `ssh`, omitindo padrões com curingas, negações e outras formas inválidas.
- Organiza hosts em grupos e subgrupos expansíveis, com tags e cores opcionais
  definidas por comentários na configuração SSH.
- Filtra hosts com busca aproximada, incluindo nomes de grupos e tags.
- Mostra os valores efetivos de HostName, User, Port, ProxyJump, ProxyCommand e
  IdentityFile obtidos com `ssh -G`.
- Restaura o terminal e executa `ssh` diretamente ao pressionar `Enter`,
  opcionalmente com `-v`, `-vv` ou `-vvv`, e devolve o código de saída do SSH ao
  shell.
- Adapta as colunas e o painel de detalhes à largura disponível no terminal.

## Instalação

O Porteiro precisa de um executável `ssh` compatível com OpenSSH na máquina em
que será executado. Para as badges arredondadas das tags, escolha uma fonte de
terminal com os semicírculos Powerline (`U+E0B6` e `U+E0B4`), como uma
[Nerd Font](https://www.nerdfonts.com/). O Porteiro não consegue detectar se a
fonte do terminal contém esses símbolos.

**Binários das releases.** Baixe o arquivo correspondente à sua plataforma na
[página de releases](https://github.com/jujubetsz/porteiro/releases), confira a
soma de verificação do arquivo `.sha256` e coloque `porteiro` no seu `PATH`:

```sh
tar -xzf porteiro-<version>-<target>.tar.gz
install -m 0755 porteiro-<version>-<target>/porteiro ~/.local/bin/porteiro
```

**A partir do código-fonte**, com uma versão estável do Rust (1.88 ou superior):

```sh
cargo install --git https://github.com/jujubetsz/porteiro --locked
```

**Nix**, sem instalar nada:

```sh
nix run github:jujubetsz/porteiro
```

## Uso

```text
porteiro [--config PATH] [--ssh PATH]
```

`--config` seleciona outro arquivo de configuração do usuário. O mesmo caminho
é passado ao OpenSSH com `-F` para consultar detalhes e realizar conexões.
`--ssh` seleciona outro executável compatível com OpenSSH e é útil quando `ssh`
não está no `PATH`.

A versão do binário em execução aparece sempre no canto superior direito,
inclusive em terminais estreitos e com painéis sobrepostos abertos.
`porteiro --version` informa a mesma versão sem abrir a TUI nem ler a
configuração SSH. Ambos os valores vêm do `Cargo.toml` e são incorporados ao
binário durante a compilação.

| Tecla | Ação |
| --- | --- |
| `j` / `k`, setas | Mover a seleção |
| `PgUp` / `PgDn`, `Home` / `End` | Navegar por página ou ir à primeira ou última linha |
| `/` | Ativar a busca |
| `Enter` | Confirmar a busca, expandir/recolher um grupo ou conectar a um host |
| `Space` | Expandir/recolher o grupo selecionado |
| `Esc` | Limpar a busca ou fechar um painel sobreposto |
| `i` | Abrir/fechar o painel de detalhes sobreposto |
| `r` | Recarregar a configuração |
| `w` | Ver avisos da configuração; `j`/`k` navegam e `Esc` fecha |
| `v` | Alternar a verbosidade da próxima conexão: `-v`, `-vv`, `-vvv`, desativada |
| `?` | Abrir/fechar a ajuda |
| `q` / `Ctrl-C` | Sair |

O host selecionado mostra uma badge com a opção de verbosidade da próxima
conexão. A dica de `Enter` e a linha de status repetem essa opção, permitindo
investigar falhas de conexão sem sair da lista.

O Porteiro encerra após o término da sessão SSH e devolve o código de saída do
processo SSH ao shell que o iniciou. Em sistemas Unix, uma sessão encerrada por
um sinal retorna o código convencional `128 + signal`.

## Grupos, tags, cores e visibilidade

Adicione comentários de linha inteira com `# porteiro:` à configuração SSH ou
aos arquivos incluídos. Essas anotações são específicas do Porteiro e não são
opções SSH. O OpenSSH
[ignora linhas de comentário](https://man.openbsd.org/ssh_config#DESCRIPTION),
por isso os mesmos arquivos continuam funcionando com `ssh`, `scp` e `sftp`.

```sshconfig
# Defaults for hosts declared in this file
# porteiro: group="Work" color=blue

Host production
    HostName prod.example.test
    User deploy
    # porteiro: tags=prod,api color=red

Host staging
    HostName staging.example.test
    # porteiro: tags=staging,api

Host personal
    HostName home.example.test
    # porteiro: group="Personal" tags=homelab color=green

Host standalone
    HostName standalone.example.test
    # porteiro: group="" color=default
```

```text
▸ Work (2)
▾ Personal (1)
└─ personal  homelab
standalone
```

- Cada host pertence a, no máximo, um grupo e pode ter qualquer quantidade de
  tags separadas por vírgulas. Coloque entre aspas os nomes que contêm espaços.
  Nomes de grupos e tags diferenciam maiúsculas de minúsculas; a busca não faz
  essa distinção. Tags repetidas aparecem uma única vez, na ordem de declaração.
- Para grupos aninhados, use `group_path="Production/APIs"`. Cada `/` introduz
  um nível; espaços nas extremidades de cada nome são removidos e componentes
  vazios são rejeitados. Por compatibilidade, `group="Production/APIs"` continua
  representando um único grupo com esse nome literal. Use apenas uma das opções
  `group` e `group_path` por anotação; ambas substituem todo o caminho herdado.
  `group_path=""` remove o agrupamento.
- Anotações antes do primeiro `Host` ou `Match` definem os valores padrão do
  arquivo. Dentro de um bloco `Host`, sobrescrevem campos dos aliases concretos
  declarados naquele bloco. A última anotação válida de cada campo prevalece;
  campos omitidos mantêm seus valores. As tags substituem as herdadas, sem se
  somar a elas. Use `group=""`, `tags=""` ou `color=default` para redefinir um campo.
- Valores padrão e anotações de blocos ficam restritos ao próprio arquivo.
  Não são propagados por `Include`, mesmo quando o arquivo incluído contém
  apenas opções SSH. A primeira declaração alcançável mantida para um alias
  fornece seus metadados; declarações duplicadas posteriores não os sobrescrevem.
- Anotações em blocos `Match` ou em blocos sem aliases concretos são ignoradas
  com um aviso. As anotações não fazem correspondência por curingas nem avaliam
  critérios de `Match`. Comentários na mesma linha, após opções SSH, continuam
  sendo comentários comuns.
- As cores são `red`, `yellow`, `green`, `blue`, `magenta`, `cyan`, `white`,
  `gray` ou `default`. Elas destacam os nomes dos hosts. O cabeçalho de um grupo
  usa a cor do primeiro host mantido nele. A seleção e os rótulos continuam
  visíveis sem depender da cor.
- As tags aparecem como cápsulas arredondadas em uma coluna `Tags` separada,
  com cores neutras e fixas para fundo e texto. As cores das badges não mudam
  com a cor do host ou com a seleção, e cada host ocupa uma única linha.
- Adicione `# porteiro: hidden=true` dentro de um bloco `Host` para ocultar seus
  aliases da lista, da busca e dos grupos do Porteiro. A configuração SSH e as
  conexões diretas com `ssh` permanecem iguais. O campo segue as mesmas regras
  de declaração e de valores padrão locais ao arquivo das outras anotações;
  `hidden=false` sobrescreve um padrão que oculta hosts. Declarações ocultas
  continuam passando pela validação da configuração.
- Anotações malformadas são ignoradas por inteiro, com diagnóstico de arquivo
  e linha disponível em `w`. Chaves ou cores desconhecidas, chaves duplicadas na
  mesma linha e caracteres de controle ou de formatação bidirecional são
  rejeitados. Erros da sintaxe SSH e de `Include` mantêm seu tratamento habitual.

Os grupos começam recolhidos, acima dos hosts sem grupo. Em cada nível, os
subgrupos seguem a ordem da primeira ocorrência e aparecem antes dos hosts
associados diretamente, que mantêm a ordem de declaração. As contagens incluem
todos os hosts descendentes. Subgrupos com nomes iguais sob grupos diferentes
expandem e recolhem de forma independente. A busca também considera caminhos
completos de grupos e tags, abre temporariamente todos os ancestrais dos hosts
encontrados e ordena os resultados dentro de cada grupo. Limpar a busca restaura
o estado anterior de expansão, inclusive dos subgrupos.
Ao recarregar, os grupos que continuam existindo mantêm seu estado de expansão
e a linha selecionada é preservada se continuar visível. O estado de expansão
é mantido apenas durante a sessão atual.

Por exemplo:

```sshconfig
Host prod-api
    # porteiro: group_path="Production/APIs" tags=api color=red
Host prod-db
    # porteiro: group_path="Production/Databases"
Host bastion
    # porteiro: group="Production"
Host staging-api
    # porteiro: group_path="Staging/APIs"
Host legacy
    # porteiro: hidden=true
```

```text
Alias                   Tags
▾ Production (3)
├─ ▾ APIs (1)
│  └─ prod-api           api
├─ ▸ Databases (1)
└─ bastion
▸ Staging (1)
```

A coluna `Tags` permanece alinhada independentemente do tamanho do alias ou da
profundidade da árvore. Ela é omitida quando nenhum host tem tags ou quando o
terminal é estreito demais para manter o alias e a opção de verbosidade
legíveis. As badges são exibidas por inteiro dentro da coluna; o indicador `+N`
informa a quantidade de tags adicionais. Pressione `i` para ver o caminho
completo do grupo e todas as tags, inclusive quando a coluna estiver oculta.
Em árvores profundas, os conectores dos ancestrais mais distantes cedem espaço
aos nomes dos hosts e às opções de conexão.

## Como funciona a descoberta de hosts

Os valores da lista são indicações obtidas pela validação do catálogo, feita
somente para leitura. Quando nenhuma opção `Port` aplicável está definida, a
lista mostra a [porta padrão do OpenSSH, 22](https://man.openbsd.org/ssh_config#Port).
Uma porta que depende de um resultado desconhecido de `Match` ou de um caminho
de `Include` permanece como `-`. O painel de detalhes mostra a configuração
efetiva: consulta o OpenSSH para o alias selecionado com `ssh -G`, com limite
de três segundos.

- O OpenSSH avalia as diretivas `Match exec` escritas pelo usuário durante essa
  consulta. Portanto, comandos presentes intencionalmente na configuração SSH
  podem ser executados ao selecionar um host. A consulta de detalhes desativa
  a canonicalização de nomes de host e CNAME para evitar tráfego DNS; a conexão
  real usa a configuração original.
- Durante a construção do catálogo, resultados de `Match` que dependem de
  comandos ou do estado da máquina são tratados como desconhecidos. Assim, não
  podem introduzir aliases especulativos nem executar comandos locais.
- Ciclos recursivos alcançáveis de `Include` são rejeitados, em conformidade
  com a falha que o OpenSSH produz ao atingir o limite de profundidade. Em Unix,
  arquivos acessados por `Include` devem atender às verificações de proprietário
  e permissões de escrita do OpenSSH.
- A validação do catálogo modela a etapa final de leitura da configuração
  solicitada pelo OpenSSH: `Host` é comparado ao host efetivo normalizado,
  enquanto `originalhost` continua sendo o alias selecionado. A etapa solicitada
  por `CanonicalizeHostname` é determinística para IPs literais; nomes cuja forma
  canônica possa exigir DNS são tratados como desconhecidos. A opção
  `CanonicalizeHostname yes` preserva um nome de host conhecido que usa proxy,
  porque o OpenSSH dispensa DNS nesse caso. Nomes com mais pontos do que o limite
  de `CanonicalizeMaxDots` não passam pela canonicalização, respeitando o valor
  padrão e a regra do OpenSSH de usar o primeiro valor obtido.
- Endereços numéricos usam o analisador da plataforma, compatível com OpenSSH e
  sem consultas DNS, e sua forma textual canônica. Isso inclui formatos legados
  de IPv4 e IPv6 com identificador de escopo. Valores de porta são normalizados
  para decimal antes da expansão de `%p` e `%C`.
- Os aliases são as formas que o OpenSSH aceita como argumento literal de host:
  padrões com curingas, negações, valores inválidos, URIs com `ssh://` em
  minúsculas e formas `user@host` são omitidos. O OpenSSH diferencia maiúsculas
  de minúsculas no esquema da URI; uma grafia com capitalização diferente
  continua sendo um alias literal.
- Separadores `=` nas diretivas são aceitos com ou sem espaços; um `=` inicial
  entre aspas continua fazendo parte do valor.

## Desenvolvimento

O código e seus comentários permanecem em inglês. O texto deste README é
mantido em português; comandos, comentários dos exemplos e saídas da interface
preservam sua forma original.

```sh
nix develop        # optional
cargo fmt --all --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
cargo build --release --all-features
```

A integração contínua (CI) executa essas verificações no Linux e no macOS,
além de `cargo check` no Windows, validação da versão mínima suportada do Rust
e auditoria de dependências.

O job `Security audit` executa `cargo audit` para procurar vulnerabilidades
conhecidas nas dependências registradas em `Cargo.lock`, usando a base
[RustSec](https://rustsec.org/). Ele roda em pull requests, em pushes na `main`
e semanalmente. O Dependabot também está configurado para propor atualizações
semanais das dependências Rust e das GitHub Actions.

Após as verificações passarem na `main`, o release-please mantém um PR de release
a partir dos commits semânticos, atualizando `Cargo.toml`, `Cargo.lock` e
`CHANGELOG.md`.

O merge do PR de release, seguido da aprovação da CI, cria a tag `vX.Y.Z` e a
release no GitHub. O mesmo pipeline compila binários para Linux x86_64 e macOS
Apple Silicon a partir dessa tag e anexa os arquivos e as somas de verificação
SHA-256. Consulte o [fluxo de releases](CONTRIBUTING.md#releases) para configurar
o repositório e repetir um envio que tenha falhado.

Consulte [CONTRIBUTING.md](CONTRIBUTING.md) para o fluxo de contribuição e
[AGENTS.md](AGENTS.md) para as regras do produto, suas invariantes e arquitetura.

## Licença

Distribuído sob a [Licença Apache, versão 2.0](LICENSE).
