import type { CSSProperties, ImgHTMLAttributes } from "react";

type Props = Omit<ImgHTMLAttributes<HTMLImageElement>, "src" | "height" | "width" | "loading"> & {
  src: string;
  alt: string;
  width?: number | string;
  height?: number | string;
  fill?: boolean;
  sizes?: string;
  priority?: boolean;
  quality?: number;
  unoptimized?: boolean;
  loading?: "eager" | "lazy";
};

export default function Image({ src, alt, width, height, fill, sizes, priority, quality, unoptimized, style, ...props }: Props) {
  const fillStyle: CSSProperties | undefined = fill
    ? { position: "absolute", inset: 0, width: "100%", height: "100%", objectFit: style?.objectFit ?? "cover" }
    : undefined;

  return (
    <img
      src={src}
      alt={alt}
      width={fill ? undefined : width}
      height={fill ? undefined : height}
      sizes={sizes}
      loading={priority ? "eager" : "lazy"}
      style={{ ...fillStyle, ...style }}
      {...props}
    />
  );
}
