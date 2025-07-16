#include <stdint.h>

struct aaa {
  int32_t a;
  int64_t b;
  bool* c;
};

struct bbb {
  aaa innards;
};

struct ccc {
  aaa* a;
};

struct ddd {
  void* (*f)(void*, int32_t);
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

eee<aaa>;

enum fff : uint8_t {
  ZERO,
  ONE,
  TWO
};

enum ggg {
  GGGONE=1,
  GGGTWO,
  GGGTHREE,
};

class HHH {
  HHH();
  HHH(int32_t);
  ~HHH();
  bool myMethod(uint32_t*);
  // ; end vtable
  uint32_t a;
  aaa* b;
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
  JJJ(); // ; override void (* HHH_vtable::HHH)(struct HHH* this);
  ~JJJ(); // ; override void (* HHH_vtable::~HHH)(struct HHH* this);
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
  bool arg2; // ; override bool III::arg1;
  void** (*cb2)(); // ; override void* (* HHH::cb)();
};

typedef MMM NNN;

typedef Abc<int32_t, bool>* OOO;

struct __attribute__((packed)) PPP {
    bool a;
    uint32_t b;
    char* c;
    aaa d;
};

namespace QQQ {
    struct aaa {
        uint32_t a;
        void* b;
    };

    namespace RRR {
        struct aaa {
            void* a;
            uint32_t b;
        };

        struct bbb {
            aaa* a;
        };

        struct ccc {
            QQQ::aaa a;
        };
    } // end RRR
} // end QQQ
