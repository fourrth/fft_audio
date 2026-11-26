import { useState, type ReactNode } from "react";
import { IconButton } from "./IconButton";
import { Banner, type BannerTone } from "./Banner";

export interface FieldProps {
  label: ReactNode;
  htmlFor?: string;
  help?: ReactNode;
  unit?: string;
  banner?: { tone: BannerTone; message: ReactNode };
  children: ReactNode;
  className?: string;
}

/** Label + optional help popover + input + mono unit + inline banner. */
export function Field({
  label,
  htmlFor,
  help,
  unit,
  banner,
  children,
  className,
}: FieldProps) {
  const [showHelp, setShowHelp] = useState(false);
  const labelText = typeof label === "string" ? label : "field";
  return (
    <div className={`field${className ? ` ${className}` : ""}`}>
      <div className="field-label">
        <label htmlFor={htmlFor}>{label}</label>
        {help ? (
          <IconButton
            label={`Help for ${labelText}`}
            className="field-help-button"
            pressed={showHelp}
            kbd={null}
            onClick={() => setShowHelp((value) => !value)}
          >
            ?
          </IconButton>
        ) : null}
      </div>
      {help && showHelp ? (
        <div className="field-help" role="note">
          {help}
        </div>
      ) : null}
      <div className="field-control">
        <div className="field-input">{children}</div>
        {unit ? <span className="field-unit">{unit}</span> : null}
      </div>
      {banner ? <Banner tone={banner.tone}>{banner.message}</Banner> : null}
    </div>
  );
}
