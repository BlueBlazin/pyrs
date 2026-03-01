(() => {
	const buttons = document.querySelectorAll("button[data-copy-button][data-copy]");

	for (const button of buttons) {
		if (!(button instanceof HTMLButtonElement)) {
			continue;
		}
		if (button.dataset.copyBound === "1") {
			continue;
		}
		button.dataset.copyBound = "1";

		const originalLabel = button.dataset.copyLabel || button.textContent?.trim() || "Copy";
		const successLabel = button.dataset.copySuccess || "Copied";
		const failureLabel = button.dataset.copyFailure || "Failed";
		const successClass = button.dataset.copySuccessClass || "is-copied";
		const failureClass = button.dataset.copyFailureClass || "is-copy-failed";
		const resetMs = Number.parseInt(button.dataset.copyResetMs || "1400", 10);

		let resetTimer;
		button.addEventListener("click", async () => {
			const payload = button.getAttribute("data-copy") || "";
			const reset = () => {
				button.textContent = originalLabel;
				button.classList.remove(successClass);
				button.classList.remove(failureClass);
			};

			button.classList.remove(successClass);
			button.classList.remove(failureClass);
			try {
				await navigator.clipboard.writeText(payload);
				button.textContent = successLabel;
				button.classList.add(successClass);
			} catch {
				button.textContent = failureLabel;
				button.classList.add(failureClass);
			}

			clearTimeout(resetTimer);
			resetTimer = window.setTimeout(reset, Number.isFinite(resetMs) ? resetMs : 1400);
		});
	}
})();
