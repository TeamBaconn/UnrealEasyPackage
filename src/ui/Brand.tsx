import { Group, Text } from "@mantine/core";
import { tokens } from "./tokens";

/** Hexagon mark + wordmark used in the gate top-bar and the main side-nav. Pass
 * `wordmark={false}` for the mark alone. */
export function Brand({ size = 22, wordmark = true }: { size?: number; wordmark?: boolean }) {
  return (
    <Group gap={10} wrap="nowrap" className="uep-chrome">
      <svg width={size} height={size} viewBox="0 0 24 24" aria-hidden="true">
        <path d="M12 2 L21 7 L21 17 L12 22 L3 17 L3 7 Z" fill="none" stroke={tokens.accent} strokeWidth={2} />
        <path d="M8 9 L16 9 L12 17 Z" fill={tokens.accent} />
      </svg>
      {wordmark && (
        <Text fw={700} fz={size > 24 ? "lg" : "md"} c={tokens.ink}>
          UnrealEasyPackage
        </Text>
      )}
    </Group>
  );
}
