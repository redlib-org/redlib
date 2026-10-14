(function () {
    function submitOnSelect(e) {
        if (e.target && e.target.form) {
            e.target.form.submit();
        }
    }

    function init() {
        const selects = document.querySelectorAll(
            '#timeframe, #sort_select, #commentSortSelect, #search_sort select[name="sort"]'
        );
        selects.forEach(function (select) {
            select.addEventListener('change', submitOnSelect);
        });
    }

    if (document.readyState === 'loading') {
        document.addEventListener('DOMContentLoaded', init);
    } else {
        init();
    }
})();
