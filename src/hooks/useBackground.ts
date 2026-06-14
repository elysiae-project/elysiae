import { useContext } from "preact/hooks";
import { BackgroundContext } from "../contexts/BackgroundContext";

export const useBackground = () => {
	const context = useContext(BackgroundContext);
	if (!context) {
		throw new Error("useBackground must be used within a BackgroundProvider");
	}
	return context;
};
