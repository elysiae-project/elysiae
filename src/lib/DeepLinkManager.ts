import { getCurrent, onOpenUrl } from "@tauri-apps/plugin-deep-link";

(async () => {
    console.log("Starting to listen to deep-link URIs")

	const startURLs = await getCurrent();
	if (startURLs) {
		console.log(`Start URIs: ${startURLs}`);
	}

	await onOpenUrl((urls) => {
		console.log(`URI(s) opened: ${urls}`);
	});
})();
