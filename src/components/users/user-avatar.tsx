import { colorClass, initialOf } from "@/lib/users/logic";
import { cn } from "@/lib/utils";

/** ユーザーのアバター(名前の頭文字 + 色。docs/36 §5.1)。 */
export function UserAvatar({
  name,
  color,
  className,
}: {
  name: string;
  color: string;
  className?: string;
}) {
  return (
    <span
      aria-hidden
      className={cn(
        "flex shrink-0 select-none items-center justify-center rounded-full font-semibold",
        colorClass(color),
        className,
      )}
    >
      {initialOf(name)}
    </span>
  );
}
