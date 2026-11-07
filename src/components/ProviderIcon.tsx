import React, { useMemo } from "react";
import { getIcon, hasIcon, getIconMetadata } from "@/icons/extracted";
import { cn } from "@/lib/utils";

interface ProviderIconProps {
  icon?: string; // Icon name
  name: string; // Provider name (used for the fallback)
  color?: string; // Custom color (Deprecated, kept for compatibility but ignored for SVG)
  size?: number | string; // Size
  className?: string;
  showFallback?: boolean; // Show the fallback
}

export const ProviderIcon: React.FC<ProviderIconProps> = ({
  icon,
  name,
  color,
  size = 32,
  className,
  showFallback = true,
}) => {
  // Icon SVG
  const iconSvg = useMemo(() => {
    if (icon && hasIcon(icon)) {
      return getIcon(icon);
    }
    return "";
  }, [icon]);

  // Size style
  const sizeStyle = useMemo(() => {
    const sizeValue = typeof size === "number" ? `${size}px` : size;
    return {
      width: sizeValue,
      height: sizeValue,
      // Inline SVGs are sized in em, so set fontSize too to make the icon follow size
      fontSize: sizeValue,
      lineHeight: 1,
    };
  }, [size]);

  // Effective color: the color prop if valid, else defaultColor from metadata
  const effectiveColor = useMemo(() => {
    // Use color only when it is a non-empty string
    if (color && typeof color === "string" && color.trim() !== "") {
      return color;
    }
    // Otherwise take defaultColor from metadata
    if (icon) {
      const metadata = getIconMetadata(icon);
      // Use defaultColor only when it is not currentColor
      if (metadata?.defaultColor && metadata.defaultColor !== "currentColor") {
        return metadata.defaultColor;
      }
    }
    return undefined;
  }, [color, icon]);

  // Show the icon if there is one
  if (iconSvg) {
    return (
      <span
        className={cn(
          "inline-flex items-center justify-center flex-shrink-0",
          className,
        )}
        style={{ ...sizeStyle, color: effectiveColor }}
        dangerouslySetInnerHTML={{ __html: iconSvg }}
      />
    );
  }

  // Fallback: show the initial
  if (showFallback) {
    const initials = name
      .split(" ")
      .map((word) => word[0])
      .join("")
      .toUpperCase()
      .slice(0, 2);
    const fallbackFontSize =
      typeof size === "number" ? `${Math.max(size * 0.5, 12)}px` : "0.5em";
    return (
      <span
        className={cn(
          "inline-flex items-center justify-center flex-shrink-0 rounded-lg",
          "bg-muted text-muted-foreground font-semibold",
          className,
        )}
        style={sizeStyle}
      >
        <span
          style={{
            fontSize: fallbackFontSize,
          }}
        >
          {initials}
        </span>
      </span>
    );
  }

  return null;
};
