import { useId, useState, type ReactNode } from "react";

export interface CardProps {
  title: ReactNode;
  subtitle?: ReactNode;
  /** Controlled open state; leave undefined for uncontrolled. */
  open?: boolean;
  defaultOpen?: boolean;
  onOpenChange?: (open: boolean) => void;
  actions?: ReactNode;
  children: ReactNode;
  className?: string;
}

/** Collapsible card used by the inspector, settings and display panels. */
export function Card({
  title,
  subtitle,
  open,
  defaultOpen = true,
  onOpenChange,
  actions,
  children,
  className,
}: CardProps) {
  const [innerOpen, setInnerOpen] = useState(defaultOpen);
  const isOpen = open ?? innerOpen;
  const bodyId = useId();

  const toggle = () => {
    const next = !isOpen;
    if (open === undefined) {
      setInnerOpen(next);
    }
    onOpenChange?.(next);
  };

  return (
    <section className={`card${isOpen ? " open" : ""}${className ? ` ${className}` : ""}`}>
      <div className="card-header">
        <button
          type="button"
          className="card-toggle"
          aria-expanded={isOpen}
          aria-controls={bodyId}
          onClick={toggle}
        >
          <span className={`card-chevron${isOpen ? " open" : ""}`} aria-hidden="true">
            ▸
          </span>
          <span className="card-heading">
            <span className="card-title">{title}</span>
            {subtitle ? <span className="card-subtitle">{subtitle}</span> : null}
          </span>
        </button>
        {actions ? <div className="card-actions">{actions}</div> : null}
      </div>
      {isOpen ? (
        <div className="card-body" id={bodyId}>
          {children}
        </div>
      ) : null}
    </section>
  );
}
