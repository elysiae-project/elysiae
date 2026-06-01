import { join } from "@tauri-apps/api/path";
import { type ComponentChildren, createContext } from "preact";
import { useEffect, useState } from "preact/hooks";
import { useGame } from "../hooks/useGame";
import { getOption } from "../lib/Settings";
import { variantToGameCode } from "../lib/VariantConverter";
import type { CachedBackgrounds } from "../types";

const ASSETS_BASE = "https://assets.elysiae.app";

interface BackgroundContextType {
	cachedBackgrounds: CachedBackgrounds | null;
	currentBackground: string | null;
}

export const BackgroundContext = createContext<BackgroundContextType>({
	cachedBackgrounds: null,
	currentBackground: null,
});

export const BackgroundProvider = ({
	children,
}: {
	children: ComponentChildren;
}) => {
	const [cachedBackgroundData, setCachedBackgroundData] =
		useState<CachedBackgrounds | null>(null);

	const [currentBackgroundPath, setCurrentBackgroundPath] = useState<
		string | null
	>(null);

	const { game } = useGame();

	useEffect(() => {
		(async () => {
			const gameCode = variantToGameCode[game];
			const backgroundDir = await join("backgrounds", gameCode);

			if (!cachedBackgroundData) {
				const cachedBackgrounds =
					await getOption<CachedBackgrounds>("cachedBackgrounds");
			}
		})();
	}, [game]);

	return (
		<BackgroundContext.Provider
			value={{
				cachedBackgrounds: cachedBackgroundData,
				currentBackground: currentBackgroundPath,
			}}
		>
			{children}
		</BackgroundContext.Provider>
	);
};
