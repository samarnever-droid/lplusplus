typedef struct Link {
    int value;
    struct Link *next;
} Link;

int link_has_next(Link *link) {
    return link->next != 0;
}

int link_next_value(Link *link) {
    return link->next->value;
}

Link *link_get_next(Link *link) {
    return link->next;
}

void link_set_next(Link *link, Link *next) {
    link->next = next;
}

void link_copy_next(Link *destination, Link *source) {
    destination->next = source->next;
}

void link_clear_next(Link *link) {
    link->next = 0;
}
