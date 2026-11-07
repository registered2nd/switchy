import type { ReactNode } from "react";
import { cn } from "@/lib/utils";

/** One titled block of settings: a heading, what it is for, then its rows. */
export function SettingsSection({
  title,
  description,
  children,
  className,
}: {
  title: ReactNode;
  description?: ReactNode;
  children: ReactNode;
  className?: string;
}) {
  return (
    <section
      className={cn("border-t border-border pt-5 first:border-t-0 first:pt-0", className)}
    >
      <div className="mb-4 max-w-[62ch]">
        <h2 className="font-display text-[17px] font-semibold tracking-tight text-foreground">
          {title}
        </h2>
        {description && (
          <p className="mt-0.5 text-[13px] leading-relaxed text-muted-foreground">
            {description}
          </p>
        )}
      </div>
      {children}
    </section>
  );
}
