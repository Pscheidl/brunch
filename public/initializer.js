export default function initializer() {
  return {
    onFailure() {
      const message = document.getElementById('loading-message');
      const retry = document.getElementById('loading-retry');
      if (message) message.textContent = 'Nabídku se nepodařilo načíst. Zkontrolujte připojení a zkuste to znovu.';
      if (retry) retry.hidden = false;
    },
  };
}
