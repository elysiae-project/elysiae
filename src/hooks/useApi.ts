import { ApiContext } from "../contexts/ApiContext";
import { useContext } from "preact/hooks";

export const useApi = () => {
	const context = useContext(ApiContext);
	if (!context) {
		throw new Error("useApi must be used within an ApiProvider");
	}
	return context;
};
