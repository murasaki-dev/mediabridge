(() => {
  const search = globalThis.ytInitialData?.contents?.twoColumnSearchResultsRenderer;
  const results = search?.primaryContents?.sectionListRenderer?.contents;
  if (!Array.isArray(results)) return {available:false,videos:[]};
  const videos = [], ids = new Set();
  const text = value => value?.simpleText || value?.runs?.map(run => run.text).join('');
  for (const section of results) {
    for (const row of section.itemSectionRenderer?.contents || []) {
      const card = row.videoRenderer;
      if (!card || Object.keys(card).some(key => /promoted|sponsored|adBadge/i.test(key))) continue;
      const id = card.videoId, title = text(card.title);
      if (!/^[A-Za-z0-9_-]{11}$/.test(id || '') || !title || ids.has(id)) continue;
      ids.add(id);
      const length = text(card.lengthText);
      const avatar = card.channelThumbnailSupportedRenderers?.channelThumbnailWithLinkRenderer?.thumbnail?.thumbnails?.at(-1)?.url;
      videos.push({
        url:'https://www.youtube.com/watch?v=' + id, title,
        uploader:text(card.ownerText || card.shortBylineText),
        published:text(card.publishedTimeText),
        avatar_url:typeof avatar === 'string' ? avatar.replace(/^\/\//,'https://') : undefined,
        duration_seconds:/^\d+(?::\d{2}){1,2}$/.test(length || '')
          ? length.split(':').reduce((total,part) => total*60+Number(part),0) : undefined,
      });
      if (videos.length === 24) return {available:true,videos};
    }
  }
  return {available:true,videos};
})()
