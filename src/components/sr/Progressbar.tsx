export default function SrProgressbar({ progress }: { progress: number }) {
	return (
		<div class="bg-[#c0bebf] h-5">
			<div
				style={{ width: `${progress}%` }}
				class="bg-[#ef973c] h-full transition-all duration-300"
			></div>
		</div>
	);
}
