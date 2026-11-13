# Arch Launcher

Lançador de aplicativos rápido e minimalista para Arch Linux, em **Rust + GTK4**,
inspirado no Rofi e no Wofi. Funciona em **Wayland** (com layer-shell em
Hyprland, Sway, KDE Plasma, river, niri…) e em **X11**.

Seu diferencial: **ocultar qualquer aplicativo direto pela interface**, sem
desinstalá-lo e sem alterar o arquivo `.desktop`. O app some **só deste
launcher** e continua no menu do sistema, no terminal e em outros launchers.

![ícone](data/icons/io.github.archlauncher.ArchLauncher.svg)

## Recursos

* Descoberta automática de apps via XDG Desktop Entry (pacman, AUR, Flatpak, `~/.local`),
  respeitando `NoDisplay`, `Hidden`, `OnlyShowIn`, `NotShowIn`, `TryExec` e a
  precedência XDG (sem duplicatas).
* Atualização ao vivo: instalar ou remover pacotes atualiza a lista sem reiniciar.
* Pesquisa fuzzy instantânea, sem diferenciar maiúsculas de minúsculas, por nome,
  palavras-chave e descrição, ordenada por relevância e frequência de uso.
* **Ocultar aplicativos**: menu de contexto (clique direito / tecla Menu) ou
  `Ctrl+Shift+X`, com "Desfazer". Persistente entre reinícios, atualizações e reconstruções de cache.
* **Gerenciador de aplicativos ocultos** (`Ctrl+Shift+H` ou menu ⚙): pesquisar, reexibir um ou todos.
* Execução segura via GIO (`GDesktopAppInfo`): sem shell, com suporte a `DBusActivatable`,
  `Terminal`, Flatpak e tokens de ativação Wayland.
* Temas claro, escuro ou do sistema, cores, transparência, fontes, tamanhos e atalhos configuráveis.
* Instância única: rodar `arch-launcher` de novo **alterna** a janela (perfeito para Super+D).
* Nada fica rodando em segundo plano (a menos que você queira usar `--daemon`).

## Instalação

### 1. Dependências (pacman)

```bash
sudo pacman -S --needed rust gtk4 gtk4-layer-shell base-devel
```

`flatpak` é opcional (só para apps Flatpak).

### 2. Compilar (release)

```bash
make
```

(equivale a `cargo build --release --locked`; o binário fica em `target/release/arch-launcher`)

### 3. Instalar

Para todo o sistema (`/usr/local`):

```bash
sudo make install
```

Ou só para o seu usuário, sem root (garanta que `~/.local/bin` está no `PATH`):

```bash
make install PREFIX=$HOME/.local
```

Ou como pacote do pacman (desinstalável com `pacman -R`):

```bash
cd packaging && makepkg -si
```

### Desinstalar

```bash
sudo make uninstall
```

(use o mesmo `PREFIX` da instalação; para o pacote: `sudo pacman -R arch-launcher`).
Suas preferências em `~/.config/arch-launcher` e `~/.local/state/arch-launcher` são mantidas;
apague essas pastas se quiser removê-las.

## Uso

```bash
arch-launcher                 # abre/fecha o launcher
arch-launcher --hidden-apps   # abre direto em "Aplicativos ocultos"
arch-launcher --daemon        # fica residente (abertura instantânea)
```

### Atalhos (dentro do launcher)

| Atalho | Ação |
|---|---|
| digitar | pesquisa instantânea |
| ↑ / ↓ (também Ctrl+P/N, Ctrl+K/J) | navegar |
| Enter | executar |
| Esc | fechar (ou voltar da página de ocultos) |
| Ctrl+Shift+X | ocultar o aplicativo selecionado |
| Ctrl+Shift+H | gerenciar aplicativos ocultos |
| Ctrl+Backspace | limpar a pesquisa |
| Menu / Shift+F10 / clique direito | menu de contexto (Abrir, Ocultar aplicativo) |

Todos podem ser alterados em `[keybindings]` no `config.toml`.

### Atalho global Super+D

O launcher **não captura atalhos globais** (no Wayland isso cabe ao compositor).
Configure o atalho no seu ambiente:

**Hyprland** (`~/.config/hypr/hyprland.conf`), exemplo completo em [examples/hyprland.conf](examples/hyprland.conf):

```ini
bind = SUPER, D, exec, arch-launcher
```

**Sway** (`~/.config/sway/config`), exemplo completo em [examples/sway.conf](examples/sway.conf):

```
bindsym $mod+d exec arch-launcher
```

**KDE Plasma**: Configurações do Sistema → Teclado → Atalhos → *Adicionar novo* →
*Comando ou script* → `arch-launcher`, e atribua Meta+D (remova antes o Meta+D padrão
de "Mostrar área de trabalho" se ele estiver em uso).

**GNOME**: Configurações → Teclado → Atalhos personalizados → comando `arch-launcher`.

**i3 / X11**:

```
bindsym $mod+d exec --no-startup-id arch-launcher
for_window [class="io.github.archlauncher.ArchLauncher"] floating enable, border none, move position center
```

**river / niri / outros**: associe qualquer tecla ao comando `arch-launcher`.

## Ocultar aplicativos

* **Ocultar:** clique direito no app → *Ocultar aplicativo*, ou selecione-o e pressione `Ctrl+Shift+X`.
  Ele some na hora e não aparece nem pesquisando o nome exato. Um aviso oferece *Desfazer*.
* **Gerenciar:** `Ctrl+Shift+H` ou ⚙ → *Aplicativos ocultos*. Dá para pesquisar, reexibir um app
  ou *Reexibir todos*. A mudança vale imediatamente.
* **Onde fica salvo:** `~/.config/arch-launcher/hidden-apps.toml`, com os **IDs XDG** dos arquivos desktop:

  ```toml
  version = 1
  hidden = ["org.mozilla.firefox.desktop", "vim.desktop"]
  ```

  A gravação é atômica (arquivo temporário + `fsync` + `rename`). Um app desinstalado continua na
  lista e, se for reinstalado, permanece oculto. Se o arquivo for corrompido, é feito um backup
  `hidden-apps.toml.corrupt-<data>` em vez de sobrescrevê-lo em silêncio.
* **Pela linha de comando** (uma janela aberta reflete a mudança na hora):

  ```bash
  arch-launcher --list            # ID, origem e nome dos apps visíveis
  arch-launcher --list --all      # inclui os ocultos
  arch-launcher --hide firefox    # por ID, ID sem .desktop ou nome exato
  arch-launcher --list-hidden
  arch-launcher --unhide org.mozilla.firefox.desktop
  arch-launcher --unhide-all
  ```

Por que não usar `NoDisplay=true`? Porque isso esconderia o app de **todos** os menus
do sistema e exigiria criar cópias de `.desktop` em `~/.local/share/applications`, que
deixam de receber atualizações do pacote. A lista própria evita os dois problemas.

## Configuração

Arquivo: `~/.config/arch-launcher/config.toml`. Ele é opcional: sem ele, valem os padrões.
Para criar um arquivo com todas as opções comentadas:

```bash
arch-launcher --print-default-config > ~/.config/arch-launcher/config.toml
```

(ou ⚙ → *Editar configuração*; ⚙ → *Recarregar configuração* aplica sem reiniciar).
Valores inválidos não impedem a inicialização: são corrigidos e registrados como aviso.
A referência completa, com os padrões, está em [examples/config.toml](examples/config.toml).

| Seção | Opções |
|---|---|
| `[appearance]` | `theme` (`dark`/`light`/`system`), `font_family`, `font_size`, `background`, `foreground`, `accent`, `selection`, `border_color`, `opacity`, `border_radius`, `border_width`, `padding`, `icon_size`, `show_descriptions`, `animations` |
| `[window]` | `width`, `height`, `anchor` (`center`/`top`), `margin_top`, `layer_shell`, `close_on_focus_loss` |
| `[behavior]` | `max_results`, `sort_by_usage`, `search_keywords`, `search_descriptions` |
| `[keybindings]` | `launch`, `close`, `next`, `previous`, `hide_selected`, `hidden_manager`, `clear_search`, `context_menu` (listas de aceleradores GTK, ex.: `"<Control><Shift>x"`) |

### CSS próprio

`~/.config/arch-launcher/style.css` é carregado por último e sobrescreve o tema gerado.
Veja [examples/style.css](examples/style.css).

### Arquivos usados

```bash
arch-launcher --paths
```

| Arquivo | Conteúdo |
|---|---|
| `~/.config/arch-launcher/config.toml` | configuração |
| `~/.config/arch-launcher/hidden-apps.toml` | apps ocultos |
| `~/.config/arch-launcher/style.css` | CSS opcional |
| `~/.local/state/arch-launcher/usage.toml` | histórico de uso (frequência) |

## Wayland × X11

| | Wayland + layer-shell (Hyprland, Sway, Plasma, river, niri) | Wayland sem layer-shell (GNOME) | X11 |
|---|---|---|---|
| Janela | sobreposição (`overlay`), nunca entra no tiling | janela comum sem bordas | janela comum sem bordas |
| Posição | centralizada (ou no topo, `anchor = "top"`) | decidida pelo compositor | decidida pelo WM (em WMs tiling, use regra de *float*) |
| Foco do teclado | exclusivo enquanto aberto | normal | normal |
| Transparência / cantos arredondados | sim | sim | só com compositor (picom, KWin, xfwm…) |
| Fechar ao perder foco | raramente se aplica (foco exclusivo) | sim | sim |
| Foco do app aberto | token `XDG_ACTIVATION_TOKEN` | idem | `DESKTOP_STARTUP_ID` |

O backend é escolhido pelo GTK. Para forçar: `GDK_BACKEND=x11 arch-launcher` ou `GDK_BACKEND=wayland arch-launcher`.
Para desativar o layer-shell: `layer_shell = false` em `[window]`.

## Desenvolvimento

```bash
cargo test                 # testes unitários e de integração
RUST_LOG=info cargo run    # executar com logs
ARCH_LAUNCHER_CONFIG_DIR=/tmp/al-cfg ARCH_LAUNCHER_STATE_DIR=/tmp/al-state cargo run   # perfil isolado
```

A arquitetura e os contratos entre módulos estão em [ARCHITECTURE.md](ARCHITECTURE.md).

## Solução de problemas

* **Super+D não faz nada:** rode `arch-launcher` no terminal; se abrir, o problema é o atalho do compositor.
* **Apps Flatpak não aparecem:** confira se `/var/lib/flatpak/exports/share` ou
  `~/.local/share/flatpak/exports/share` existem. O launcher os adiciona mesmo que o compositor
  não exporte `XDG_DATA_DIRS`.
* **A janela aparece no tiling (Hyprland/Sway):** o compositor não oferece layer-shell, ou
  `layer_shell = false`. Use as regras de *float* dos exemplos.
* **Alguns ícones aparecem genéricos no KDE:** o GTK está usando o tema Adwaita, que não
  tem os ícones do Breeze. Configure o tema de ícones do GTK em
  `~/.config/gtk-4.0/settings.ini`:

  ```ini
  [Settings]
  gtk-icon-theme-name=breeze-dark
  ```

  (ou em Configurações do Sistema → Aparência → Estilo de aplicativos → GTK).
* **Logs:** `RUST_LOG=debug arch-launcher`.
* **Captura da janela sem ferramenta de screenshot** (desenvolvimento):
  `ARCH_LAUNCHER_DEBUG_SNAPSHOT=/tmp/l.png ARCH_LAUNCHER_DEBUG_QUERY=fire arch-launcher`
  renderiza a janela em PNG pelo próprio GTK e encerra.

## Licença

MIT. Veja [LICENSE](LICENSE).
