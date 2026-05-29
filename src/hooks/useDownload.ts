import { useContext } from "preact/hooks";
import { DownloadContext } from "../contexts/DownloadContext";

export const useDownload = () => useContext(DownloadContext);
