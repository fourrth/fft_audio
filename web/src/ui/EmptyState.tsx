import type { ReactNode } from "react";

export interface EmptyStateProps {
  icon?: ReactNode;
  title: string;
  description?: ReactNode;
  actions?: ReactNode;
  hint?: ReactNode;
  className?: string;
}

/** Friendly empty state shared by launcher and display frames. */
export function EmptyState({
  icon,
  title,
  description,
  actions,
  hint,
  className,
}: EmptyStateProps) {
  return (
    <section className={`empty-state${className ? ` ${className}` : ""}`}>
      {icon ? (
        <div className="empty-icon" aria-hidden="true">
          {icon}
        </div>
      ) : null}
      <h2>{title}</h2>
      {description ? <p>{description}</p> : null}
      {actions ? <div className="empty-actions">{actions}</div> : null}
      {hint ? <p className="empty-hint">{hint}</p> : null}
    </section>
  );
}
