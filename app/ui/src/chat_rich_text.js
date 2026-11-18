mermaid.initialize({startOnLoad:false,securityLevel:'strict',theme:'base',fontFamily:'-apple-system, sans-serif',themeVariables:{primaryColor:'#eeeeff',primaryBorderColor:'#9490c8',primaryTextColor:'#292839',lineColor:'#8b88a8'},suppressErrorRendering:true});
const answer = document.getElementById('answer');
let revision = 0;
function measure(){ document.title = 'stology:' + Math.max(24, Math.ceil(answer.getBoundingClientRect().height)); }
new ResizeObserver(measure).observe(answer);
window.presentAnswer = async function(html, finished){
  const current = ++revision;
  answer.innerHTML = html;
  measure();
  if (!finished) return;
  for (const source of Array.from(answer.querySelectorAll('.diagram-source'))) {
    const code = source.textContent;
    const wrapper = document.createElement('section'); wrapper.className='diagram';
    const details = document.createElement('details');
    const summary = document.createElement('summary'); summary.textContent='차트 원문';
    details.append(summary,source.cloneNode(true));
    try {
      const {svg} = await mermaid.render('chart'+current+'x'+Math.random().toString(36).slice(2),code);
      if (current !== revision) return;
      const drawing = document.createElement('div'); drawing.innerHTML=svg;
      wrapper.append(drawing,details); source.replaceWith(wrapper);
    } catch (_) {
      if (current !== revision) return;
      const notice=document.createElement('div'); notice.className='notice'; notice.textContent='차트를 표시하지 못했어요. 원문을 확인해 주세요.';
      details.open=true; wrapper.append(notice,details);source.replaceWith(wrapper);
    }
    measure();
  }
};
// No navigation, including links embedded in diagram labels.
document.addEventListener('click', e => { if(e.target.closest('a')) e.preventDefault(); });
measure();
