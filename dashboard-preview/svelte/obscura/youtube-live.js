(() => {
  const chat = globalThis.ytInitialData?.contents;
  if (chat?.liveChatRenderer) return {live:chat.liveChatRenderer.isReplay !== true};
  if (chat?.messageRenderer) return {live:false};
  return {live:null};
})()
