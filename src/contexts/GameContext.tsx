import { createContext, ComponentChildren } from "preact";
import { useState } from "preact/hooks";
import { Variants } from "../types";

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
	return (
		<GameContext.Provider value={{ game, setGame }}>
			{children}
		</GameContext.Provider>
	);
};
