import { DownloadContext } from "../contexts/DownloadContext";
import { useContext } from "preact/hooks";

export const useDownload = () => useContext(DownloadContext);
