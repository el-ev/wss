// Upstream reference: rui314/8cc@b480958 test/pointer.c (t6)
struct Node {
  int val;
  struct Node *next;
};

int _start(void) {
  struct Node node1 = {1, 0};
  struct Node node2 = {2, &node1};
  struct Node node3 = {3, &node2};
  struct Node *p = &node3;

  int chain = p->val * 100 + p->next->val * 10 + p->next->next->val;
  p->next = p->next->next;

  return chain + p->next->val;
}
