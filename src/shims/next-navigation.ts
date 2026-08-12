import { useSyncExternalStore } from "react";

const ROUTE_EVENT = "aloe:desktop-route";

const subscribe = (listener: () => void) => {
  window.addEventListener("popstate", listener);
  window.addEventListener(ROUTE_EVENT, listener);
  return () => {
    window.removeEventListener("popstate", listener);
    window.removeEventListener(ROUTE_EVENT, listener);
  };
};

const currentPath = () => window.location.pathname;

/* Every entry carries its own depth. A browser hands you back/forward buttons
   for free; a Tauri window has no chrome at all, so the title bar draws them —
   and it can only know whether they are live if each entry is stamped on the
   way in. `furthest` is the far end of the stack: a push from the middle of it
   truncates whatever was ahead, which is exactly when forward has to go dead. */
type RouteState = { aloeIndex?: number } | null;

const stateIndex = () => (window.history.state as RouteState)?.aloeIndex ?? 0;

let furthest = 0;

export const navigateTo = (href: string, replace = false) => {
  const url = new URL(href, window.location.origin);
  const index = replace ? stateIndex() : stateIndex() + 1;
  const target = `${url.pathname}${url.search}${url.hash}`;

  window.history[replace ? "replaceState" : "pushState"]({ aloeIndex: index }, "", target);
  furthest = replace ? Math.max(furthest, index) : index;
  window.dispatchEvent(new Event(ROUTE_EVENT));
};

/** Where this entry sits in the session stack, for the title bar's back/forward. */
export const routePosition = () => {
  const index = stateIndex();
  return { index, furthest: Math.max(furthest, index) };
};

export const goBack = () => window.history.back();
export const goForward = () => window.history.forward();

export const subscribeToRoute = subscribe;

export const usePathname = () => useSyncExternalStore(subscribe, currentPath, () => "/app/chat");

export const useParams = <T extends Record<string, string | string[] | undefined> = Record<string, string | string[] | undefined>>(): T => {
  const pathname = useSyncExternalStore(subscribe, currentPath, () => "/app/chat");
  const match = pathname.match(/^\/app\/chat\/([^/]+)$/);
  return (match ? { conversationId: decodeURIComponent(match[1]) } : {}) as T;
};

export const useSearchParams = () => {
  useSyncExternalStore(subscribe, () => window.location.search, () => "");
  return new URLSearchParams(window.location.search);
};

export const useRouter = () => ({
  push: (href: string) => navigateTo(href),
  replace: (href: string) => navigateTo(href, true),
  back: goBack,
  forward: goForward,
  refresh: () => window.dispatchEvent(new Event(ROUTE_EVENT)),
});

export const redirect = (href: string): never => {
  navigateTo(href, true);
  throw new Error(`Redirected to ${href}`);
};
