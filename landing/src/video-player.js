import './video-player.css';

// A passive recording: choosing a feature is the only playback interaction.
export function createVideoPlayer(element, initialItem) {
  const video = element.querySelector('video');
  const error = element.querySelector('.recording-error');
  let itemId, visible = false, destroyed = false;

  video.controls = false;
  video.muted = true;
  video.defaultMuted = true;
  video.disablePictureInPicture = true;
  video.disableRemotePlayback = true;

  const syncPlayback = () => {
    if (destroyed || !visible || document.hidden) {
      video.pause();
      return;
    }
    // Browser autoplay policies can reject play(). The poster remains visible.
    const playing = video.play();
    playing?.catch(() => {});
  };
  const preventInteraction = event => event.preventDefault();
  const onError = () => { error.hidden = false; };
  const onLoaded = () => { error.hidden = true; syncPlayback(); };
  const observer = new IntersectionObserver(entries => {
    visible = entries[0].isIntersecting;
    syncPlayback();
  }, { threshold: .05 });

  const setItem = item => {
    if (item.id === itemId) return;
    itemId = item.id;
    video.pause();
    error.hidden = true;
    video.dataset.item = item.id;
    video.poster = `/media/demos/${item.id}.jpg`;
    video.src = `/media/demos/${item.id}.mp4`;
    video.load();
    syncPlayback();
  };

  element.addEventListener('contextmenu', preventInteraction);
  video.addEventListener('loadeddata', onLoaded);
  video.addEventListener('canplay', syncPlayback);
  video.addEventListener('error', onError);
  document.addEventListener('visibilitychange', syncPlayback);
  observer.observe(element);
  setItem(initialItem);

  return {
    setItem,
    destroy() {
      destroyed = true;
      observer.disconnect();
      video.pause();
      element.removeEventListener('contextmenu', preventInteraction);
      video.removeEventListener('loadeddata', onLoaded);
      video.removeEventListener('canplay', syncPlayback);
      video.removeEventListener('error', onError);
      document.removeEventListener('visibilitychange', syncPlayback);
      video.removeAttribute('src');
      video.load();
    }
  };
}
