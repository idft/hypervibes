(function () {
    "use strict";

    function renderIn(root) {
        if (typeof timeago === "undefined" || !root || !root.querySelectorAll) {
            return;
        }
        var nodes = root.querySelectorAll("time.timeago");
        if (nodes.length > 0) {
            timeago.render(nodes);
        }
    }

    function init() {
        renderIn(document);

        if (typeof MutationObserver === "undefined") {
            return;
        }
        var observer = new MutationObserver(function (mutations) {
            for (var i = 0; i < mutations.length; i++) {
                var added = mutations[i].addedNodes;
                for (var j = 0; j < added.length; j++) {
                    var node = added[j];
                    if (!(node instanceof Element)) {
                        continue;
                    }
                    if (node.matches("time.timeago")) {
                        timeago.render(node);
                    }
                    renderIn(node);
                }
            }
        });
        observer.observe(document.documentElement, {
            childList: true,
            subtree: true,
        });
    }

    if (document.readyState === "loading") {
        document.addEventListener("DOMContentLoaded", init);
    } else {
        init();
    }
})();
