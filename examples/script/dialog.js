document.addEventListener("DOMContentLoaded", function() {
  var quotes = document.querySelectorAll("blockquote");
  quotes.forEach((q) => {
    q.className = 'dialog';
    var prev = q.previousElementSibling;
    if (prev.tagName == "P") {
      prev.className = "speaker";
    }
    else {
      console.log(`Prev: ${prev.tagName}`);
    }
  });
});
