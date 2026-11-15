import { Button as SdkButton } from "@ui-sdk/react/button";
import type { ComponentProps, ReactNode } from "react";

type Variant = "primary" | "secondary" | "ghost" | "danger";
type Size = "sm" | "md";

type Props = Omit<ComponentProps<typeof SdkButton>, "children" | "className"> & {
  variant?: Variant;
  size?: Size;
  icon?: ReactNode;
  busy?: boolean;
  className?: string;
  children?: ReactNode;
};

const VARIANT: Record<Variant, string> = {
  primary: "border-fg bg-fg text-ink-0 enabled:hover:border-fg-2 enabled:hover:bg-fg-2",
  secondary: "border-line-2 bg-transparent text-fg enabled:hover:border-fg",
  ghost: "border-transparent bg-transparent text-fg-2 enabled:hover:bg-ink-2 enabled:hover:text-fg",
  danger: "border-hot bg-transparent text-hot enabled:hover:bg-hot enabled:hover:text-ink-0",
};
const SIZE: Record<Size, string> = { sm: "h-7 px-2.5 text-[13px] gap-1.5", md: "h-8 px-3 text-[13px] gap-2" };
const SQUARE: Record<Size, string> = { sm: "w-7 !px-0", md: "w-8 !px-0" };

export function Button({ variant = "secondary", size = "md", icon, busy, disabled, className = "", children, ...rest }: Props) {
  return (
    <SdkButton
      {...rest}
      disabled={disabled || busy}
      aria-busy={busy || undefined}
      data-variant={variant === "danger" ? "destructive" : variant === "primary" ? "solid" : undefined}
      className={`ui-button inline-flex shrink-0 select-none items-center justify-center whitespace-nowrap rounded-none border font-medium leading-none outline-offset-2 transition-colors duration-150 focus-visible:outline focus-visible:outline-1 focus-visible:outline-fg disabled:cursor-not-allowed disabled:opacity-40 data-[pending]:cursor-progress data-[pending]:opacity-60 ${VARIANT[variant]} ${SIZE[size]} ${icon && !children ? SQUARE[size] : ""} ${busy ? "cursor-progress" : ""} ${className}`}
    >
      {icon && <span className="inline-flex shrink-0 [&>svg]:size-4" aria-hidden="true">{icon}</span>}
      {children}
    </SdkButton>
  );
}
