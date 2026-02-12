export default function YsTitlebar({ children }: { children: any }) {
	return (
		<div
			data-tauri-drag-region
			class="bg-ys-titlebar font-ys h-15 min-w-full p-1"
		>
			<div
				data-tauri-drag-region
				class="flex flex-row justify-between items-center border border-[#505869] bg-transparent px-5 py-1.5"
			>
				<h3 class="text-xl text-center" data-tauri-drag-region>
					Yoohoo!
				</h3>
				{children}
			</div>
		</div>
	);
}
