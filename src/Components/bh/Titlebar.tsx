export default function Titlebar() {
	return (
		<div
			data-tauri-drag-region
			class="bg-bh-titlebar w-full h-auto rounded-t-md flex flex-row justify-between font-hsr-hi3 px-5 py-3"
		>
			<h3 class="text-xl" data-tauri-drag-region>
				Yoohoo!
			</h3>
		</div>
	);
}
