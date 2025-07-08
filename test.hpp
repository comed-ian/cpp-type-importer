#include <stdint.h>

struct Zyx_abc {
  int32_t a;
  int64_t b;
  bool* c;
};

struct ddd {
  void* (*f)(void*, int32_t);
};

struct aaa {
  Zyx_abc innards;
};

struct ccc {
  Zyx_abc* a;
};

template <typename T, typename UV> struct Abc {
  T* a;
  UV* *b;
};

Abc<int32_t, bool>;

Abc<Abc<int32_t, bool>, int32_t>;

Abc<uint32_t, uint32_t*>;

template <typename A> struct Def {
  Abc<A, A*> a;
};

Def<uint32_t>;

template <typename T> struct eee {
  void* (*fn)(T* clazz);
};

eee<Zyx_abc>;

enum fff : uint8_t {
  ZERO,
  ONE,
  TWO
};

enum ggg {
  GGGZERO=1,
  GGGONE,
  GGGTWO
};

class HHH {
  HHH();
  HHH(int32_t);
  ~HHH();
  bool myMethod(uint32_t*);
  // ; end vtable
  uint32_t a;
  Zyx_abc* b;
  void* (*cb)();
};

class III {
  III();
  ~III();
  bool MyMethod(uint32_t* a); // ; offset=1
  // ; end vtable
  bool arg1;
  char* unk;
};

class JJJ : HHH {
  // ; end vtable
  uint32_t jjj_member;
};

class KK : HHH, III {
  // ; end vtable
  uint32_t kk_member;
};

class LLL {
  LLL();
  ~LLL();
  bool testNum(ggg num); // ; offset=1
  // ; end vtable
  ggg num;
};

class MMM : HHH, III, LLL {
  uint32_t getVal();
  // ; end vtable
  uint32_t MMM_val;
};
