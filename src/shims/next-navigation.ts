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

const navigate = (href: string, replace = false) => {
  const url = new URL(href, window.location.origin);
  window.history[replace ? "replaceState" : "pushState"]({}, "", `${url.pathname}${url.search}${url.hash}`);
  window.dispatchEvent(new Event(ROUTE_EVENT));
};

export const usePathname = () => useSyncExternalStore(subscribe, currentPath, () => "/app/chat");

export const useSearchParams = () => {
  useSyncExternalStore(subscribe, () => window.location.search, () => "");
  return new URLSearchParams(window.location.search);
};

export const useRouter = () => ({
  push: (href: string) => navigate(href),
  replace: (href: string) => navigate(href, true),
  back: () => window.history.back(),
  refresh: () => window.dispatchEvent(new Event(ROUTE_EVENT)),
});

export const redirect = (href: string): never => {
  navigate(href, true);
  throw new Error(`Redirected to ${href}`);
};
