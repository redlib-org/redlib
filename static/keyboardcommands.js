document.addEventListener("DOMContentLoaded", function() {

allComments = [];
topmostCommentsInThreads = [];
let parentCommentIndex = 0;
let topmostCommentIndex = 0;

const COMMENT_INDEX_INCREMENT = 1;
const COMMENT_INDEX_DECREMENT = -1;

document.querySelectorAll('.thread .comment').forEach(element => {
    allComments.push(element.getAttribute('data-comment-id'));

});

document.querySelectorAll('.thread').forEach((thread) => {
    const allCommentsInThread = thread.querySelectorAll('.comment');
    allCommentsInThread.forEach((comment) => {
      if (!comment.closest('blockquote')) {
        topmostCommentsInThreads.push(comment);
      }
    });
  });


  document.addEventListener('keypress', (event) => {
    if (event.key === 'Enter' && event.shiftKey) {
      window.open(post_url.href, "_blank").focus();//open link in new tab, though in Edge it seems to be new window
    } else if (event.key === 'Enter') {
      window.location.href = post_url.href;
    } else if (event.key === 'j') {//next comment
      const nextComment = document.getElementById(allComments[parentCommentIndex + COMMENT_INDEX_INCREMENT]);
      parentCommentIndex += COMMENT_INDEX_INCREMENT;
      nextComment.scrollIntoView({ behavior: "smooth", block: "start" });
    } else if (event.key === 'k') {//previous comment
      const previousComment = document.getElementById(allComments[parentCommentIndex + COMMENT_INDEX_DECREMENT]);
      parentCommentIndex += COMMENT_INDEX_DECREMENT;
      if (parentCommentIndex < 0) {
        parentCommentIndex = 0;
      }
      previousComment.scrollIntoView({ behavior: "smooth", block: "start" });
    } else if (event.key=== 't') { //top of next thread
      const nextTopmostComment = topmostCommentsInThreads[topmostCommentIndex + COMMENT_INDEX_INCREMENT];
      topmostCommentIndex += COMMENT_INDEX_INCREMENT;
      nextTopmostComment.scrollIntoView({ behavior: "smooth", block: "start" });
    } else if (event.key === 'p') {//top of previous thread
      const previousTopmostComment = topmostCommentsInThreads[topmostCommentIndex + COMMENT_INDEX_DECREMENT];
      topmostCommentIndex += COMMENT_INDEX_DECREMENT;
      if (topmostCommentIndex < 0) {
        topmostCommentIndex = 0;
      }
      previousTopmostComment.scrollIntoView({ behavior: "smooth", block: "start" });
    }
  });
});
