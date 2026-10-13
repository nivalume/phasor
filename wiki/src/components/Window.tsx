import type { ReactNode } from "react";

/** A chat-client window: icon square, channel title, window controls, then content. */
export function Window({
  icon = "#",
  iconColor,
  title,
  subtitle,
  tools,
  children,
  className = "",
  dotted = false,
  as: Tag = "section",
  labelledBy,
}: {
  icon?: ReactNode;
  iconColor?: string;
  title: ReactNode;
  subtitle?: ReactNode;
  tools?: ReactNode;
  children: ReactNode;
  className?: string;
  dotted?: boolean;
  as?: "section" | "aside" | "nav" | "article" | "div";
  labelledBy?: string;
}) {
  const win = (
    <Tag className={`window${dotted ? " window--dotted" : ""} ${className}`} aria-labelledby={labelledBy}>
      <div className="window__bar">
        <span className="window__icon" style={iconColor ? { background: iconColor } : undefined} aria-hidden="true">
          {icon}
        </span>
        <div className="window__title" id={labelledBy}>
          {title}
          {subtitle ? <small>{subtitle}</small> : null}
        </div>
        <div className="window__tools">
          {tools}
          <span className="window__dots" aria-hidden="true">
            <i />
            <i />
            <i />
          </span>
        </div>
      </div>
      {children}
    </Tag>
  );
  // The halftone shadow sits at z-index -1; the wrapper is its stacking context, so it stays above the page background.
  return dotted ? <div className="window-shell">{win}</div> : win;
}
