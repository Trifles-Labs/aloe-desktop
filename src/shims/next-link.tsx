import type { AnchorHTMLAttributes, MouseEvent, ReactNode } from "react";
import { navigateTo } from "./next-navigation";

type Props = Omit<AnchorHTMLAttributes<HTMLAnchorElement>, "href"> & {
  href: string | { pathname?: string };
  children?: ReactNode;
};

export default function Link({ href, onClick, children, ...props }: Props) {
  const target = typeof href === "string" ? href : href.pathname ?? "/";

  const handleClick = (event: MouseEvent<HTMLAnchorElement>) => {
    onClick?.(event);
    if (event.defaultPrevented || event.button !== 0 || event.metaKey || event.ctrlKey || event.shiftKey || event.altKey || props.target === "_blank") return;
    event.preventDefault();
    // Same entry point as useRouter().push, so link clicks land in the session
    // stack with a depth stamp and the title bar's history controls stay honest.
    navigateTo(target);
  };

  return <a {...props} href={target} onClick={handleClick}>{children}</a>;
}
