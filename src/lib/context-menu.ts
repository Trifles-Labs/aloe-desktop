/* The webviews are windows, not browser tabs.
 *
 * WebView2 offers its page menu on right-click: Back, Refresh, Save as, Print,
 * More tools and Inspect. None of it means anything in a desktop window, and
 * because the `devtools` feature is compiled into release builds, Inspect there
 * opens the inspector on a shipped app.
 *
 * There is no app menu to replace it with yet, so right-click is inert. Copy and
 * paste keep working through the keyboard shortcuts, which is how they are used
 * in a chat composer anyway. If a menu is ever wanted, this is the one place to
 * build it. */
export function disableBrowserContextMenu(): void {
  window.addEventListener("contextmenu", (event) => event.preventDefault());
}
