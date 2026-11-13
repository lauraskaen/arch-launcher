# Arquitetura do arch-launcher

Launcher de aplicativos em Rust + GTK4 para Arch Linux, focado em Wayland, com
X11 suportado. O requisito central é **ocultar aplicativos só neste launcher**,
sem desinstalá-los e sem mexer nos seus arquivos `.desktop`.

## Decisão técnica principal: lista própria de ocultos

O launcher **não** escreve `NoDisplay=true` em cópias de `.desktop` em
`~/.local/share/applications`. Ele mantém uma lista própria em
`~/.config/arch-launcher/hidden-apps.toml`, com os **IDs XDG** dos arquivos
desktop (`org.mozilla.firefox.desktop`):

* o app some **só** do arch-launcher: continua no menu do KDE/GNOME, no
  `rofi`, no `wofi`, etc.;
* nada exige root e nenhum arquivo de terceiros é alterado;
* o ID é estável entre atualizações de pacote, mudanças de idioma e
  reconstrução de cache (o nome exibido não é);
* IDs de apps desinstalados são **mantidos**: se o app voltar, continua oculto;
* toda alteração é gravada de forma atômica *antes* de ser confirmada; se a
  gravação falhar, o estado em memória é revertido.

## Módulos

```
src/
├── main.rs              CLI (--list, --hide, --unhide, --launch, …) e entrada da GUI
├── lib.rs               mapa de módulos
├── paths.rs             caminhos XDG (config, state) + overrides para testes
├── atomic.rs            gravação atômica (tmp + fsync + rename, segue symlinks)
├── discovery/           Application Discovery
│   ├── desktop_file.rs  parser do grupo [Desktop Entry] (puro Rust, Send)
│   └── mod.rs           varredura XDG, precedência, NoDisplay/Hidden/OnlyShowIn/NotShowIn/TryExec
├── visibility.rs        Visibility Manager (hidden-apps.toml)
├── store.rs             AppStore: discovery + visibility + monitoramento ao vivo
├── launcher.rs          Launcher Service (GDesktopAppInfo, nunca shell)
├── platform.rs          Wayland (layer-shell) / X11
├── app.rs               ciclo de vida: instância única, toggle, modo daemon
├── config.rs            Configuration Manager (config.toml)
├── theme.rs             Theme Manager (CSS gerado + style.css do usuário)
├── search.rs            Search Engine (fuzzy, nucleo-matcher)
├── usage.rs             histórico de uso (frecência)
└── ui/                  UI Components (janela, lista, menu de contexto, gerenciador de ocultos)
```

### Fluxo de dados

```
 XDG dirs ──scan (thread)──▶ Vec<AppEntry> ─┐
                                            ├─▶ AppStore ──visible_apps()──▶ SearchEngine ──▶ UI
 hidden-apps.toml ──▶ VisibilityManager ────┘        ▲  │
         ▲                                           │  └─ connect_changed(StoreEvent)
         └── GFileMonitor (CLI / outra instância) ───┘
```

**Invariante:** a UI e a busca só recebem `AppStore::visible_apps()`. Um app
oculto nunca chega ao índice de busca, então não aparece nem pesquisando o nome exato.

## Contratos entre módulos

### `discovery`
* `DiscoveryOptions::from_env()` — data dirs em ordem de precedência
  (`$XDG_DATA_HOME`, Flatpak exports se faltarem no ambiente, `$XDG_DATA_DIRS`).
* `discover(&opts) -> Vec<AppEntry>` — deduplicado por ID (o primeiro diretório
  vence, mesmo que esteja `Hidden`/`NoDisplay`, que é o mecanismo de máscara
  do XDG), ordenado por nome.
* `AppEntry { id, path, name, generic_name, comment, keywords, categories, icon, exec, terminal, dbus_activatable, source }`.

### `visibility::VisibilityManager`
`load(path) -> (Self, Option<LoadWarning>)`, `is_hidden`, `hide`, `unhide`,
`unhide_all`, `unhide_many`, `forget_missing`, `reload`, `partition`.

### `store::AppStore` (clonável, `Rc`, thread principal)
* consultas: `visible_apps()`, `hidden_apps()`, `orphan_hidden_ids()`, `get(id)`, `is_hidden(id)`;
* mutações: `hide(id)`, `unhide(id)`, `unhide_all()` → persistem e emitem `StoreEvent::VisibilityChanged`;
* `watch()` → `GFileMonitor` nos diretórios `applications/` (debounce de 300 ms,
  varredura em thread) e no `hidden-apps.toml`; emite `StoreEvent::AppsChanged`;
* `connect_changed(Fn(StoreEvent))`.

### `launcher`
`launch_with_callback(entry, ctx, done)` usa `GDesktopAppInfo::launch_uris_async`:
`Exec` é dividido com as regras de aspas do `g_shell_parse_argv` (nenhum shell é
iniciado), os field codes são expandidos e `DBusActivatable`, `Terminal`,
`Path`, `StartupNotify` e o token de ativação XDG (Wayland) /
`DESKTOP_STARTUP_ID` (X11) são respeitados. Flatpaks rodam pelo próprio
`Exec=flatpak run …`.

### `app::AppContext` (entregue à UI)
* `store`, `config: RefCell<Config>`, `backend`, `resident`;
* `launch(&entry, on_error)` — chame **antes** de esconder a janela (o token de
  ativação é capturado de forma síncrona); se não estiver residente, encerra o processo após o launch;
* `dismissed()` — chame depois de esconder a janela quando o usuário fecha o launcher.

### `ui::Ui` (contrato usado por `app.rs`)
```rust
impl Ui {
    pub fn new(ctx: Rc<AppContext>) -> Rc<Ui>;
    pub fn present_launcher(&self);
    pub fn present_hidden_manager(&self);
    pub fn toggle(&self);
}
```

### `config`
`Config::load() -> (Config, Vec<String>)` nunca falha (valores inválidos são
corrigidos e geram avisos); `default_config_toml() -> String` gera o arquivo padrão comentado.

## Wayland × X11

| | Wayland + layer-shell (Hyprland, Sway, Plasma, river, niri) | Wayland sem layer-shell (GNOME) | X11 |
|---|---|---|---|
| Superfície | camada `overlay`, sem decoração | toplevel sem decoração | toplevel sem decoração |
| Posição | centralizada (ou ancorada no topo + margem) | decidida pelo compositor | decidida pelo WM (regra de float em WMs tiling) |
| Teclado | foco exclusivo | foco normal | foco normal |
| Fechar ao perder foco | raramente necessário (foco exclusivo) | sim | sim |
| Atalho global | configurado no compositor | configurado no compositor | configurado no WM / sxhkd |

O launcher nunca captura atalhos globais. `arch-launcher` é instância única
(D-Bus): rodar de novo **alterna** a janela, então Super+D basta.

## Ciclo de vida
* Padrão: o processo termina ao fechar o launcher ou ao iniciar um app. Nada fica rodando em segundo plano.
* `--daemon`: fica residente com a janela oculta, para abrir instantaneamente.
