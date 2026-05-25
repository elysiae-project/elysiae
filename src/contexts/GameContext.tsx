import { warn } from "@tauri-apps/plugin-log";
import { type ComponentChildren, createContext } from "preact";
import { useEffect, useState } from "preact/hooks";
import { type GameCodes, Variants } from "../types";
import { gameCodeToVariant, variantToGameCode } from "../util/AppFunctions";
import { getOption, setOption } from "../util/Settings";

interface GameContextType {
	game: Variants;
	setGame: (variant: Variants) => void;
}

export const GameContext = createContext<GameContextType>({
	game: Variants.NAP,
	setGame: () => {},
});

export const GameProvider = ({ children }: { children: ComponentChildren }) => {
	const [game, setGame] = useState<Variants>(Variants.HKRPG);
	useEffect(() => {
		(async () => {
			const lastSelectedGame = gameCodeToVariant[await getOption<string>("selectedGame") as GameCodes]
			if (lastSelectedGame) {
				setGame(lastSelectedGame);
			} else {
				warn("GameProvider: selectedGame option is missing, null, or empty");
			}
		})();
	}, []);

	useEffect(() => {
		(async () => {
			const gameCode = variantToGameCode[game];
			await setOption("selectedGame", gameCode);
		})();
	}, [game]);

	return (
		<GameContext.Provider value={{ game, setGame }}>
			{children}
		</GameContext.Provider>
	);
};
